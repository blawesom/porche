# 03 — Stack recommandée (catégorie 4)

Document de travail — v2, 2026-10-17. Objectif : référence d'architecture de l'orchestrateur de cloud maison — data plane d'exécution isolée (microVMs) pour workloads non fiables (agents IA, jobs, builds), densité raisonnable, GPU en phase 2.

## Architecture de l'orchestrateur

**Data plane (par hôte)**
- Kata Containers 4.x + Cloud Hypervisor sur containerd — chaque workload = microVM (guest kernel dédié, device model minimal)
- Hardening hôte (section dédiée) — obligatoire sur tous les hôtes du pool
- Image pipeline : rootfs guest immuable (base minimaliste + kata-agent), tags de version

**Control plane**
- API : création/détruction/état des microVMs, quotas, labels
- Scheduler : bin-packing CPU/RAM, topology-aware GPU (IOMMU groups), anti-affinité
- Réseau : ns réseau par VM, NAT egress par défaut, pas de port hôte exposé
- Stockage : volumes éphémères locaux ; persistentes (Ceph/NFS) en phase 2
- Metering : heures CPU/RAM/GPU par workload

**Phasage**
- **P0 — Pilote** : 1 hôte, 20 microVMs, exécution du plan de validation ci-dessous
- **P1 — Pool** : N hôtes, control plane, image pipeline, multi-utilisateurs
- **P2 — GPU** : passthrough VFIO (ou MIG) + scheduler GPU
- **P3 — Fiabilité** : HA control plane, migrations, alerting

## Recommandation

**Kata Containers 4.x + Cloud Hypervisor, sur containerd (ou Docker)**

Pourquoi :
- API container inchangée (mêmes images, même workflow) ; chaque sandbox = microVM avec guest kernel dédié
- Cloud Hypervisor : VFIO → **passthrough GPU plus tard sans changer de stack** (Firecracker = zéro GPU)
- Indépendance structurelle maximale parmi les options à faible friction
- Le VMM Cloud Hypervisor est le plus audité des deux (Rust, fuzzing rust-vmm)

**Quand choisir autre chose :**
| Besoin | Choix |
|--------|-------|
| Densité max (1000+ microVMs/node), device model minimal, zéro GPU | Firecracker + jailer (glue code à écrire : rootfs, réseau, lifecycle) |
| Contrôle total du VMM avec GPU dès le départ | Cloud Hypervisor directement (même glue) |
| Prototypage rapide sur machine de dev | Docker Sandbox (B- en produit, 3 CVE 2026) |
| Écosystème Kubernetes existant | Kata sous K8s (kata-runtime comme CRI runtime) |

À éviter : QEMU microVM — hérite de la surface cat 5 (volume CVE élevé, corrections parfois incomplètes, cf. CVE-2024-7730 → CVE-2026-3195).

## Prérequis hôte

- `/dev/kvm` disponible (bare metal ou VM avec KVM/nested virt activé — **à vérifier sur les instances cloud avant déploiement**)
- Kernel hôte ≥ 6.x récent (les CVE « page-cache » de 2026 exigent un kernel patché)
- ~64 Mo RAM + 1 vCPU de soubassement par microVM (budget à dimensionner)

## Installation (Debian/Ubuntu-like)

```bash
# Kata static (VMM cloud-hypervisor inclus)
curl -sSL https://github.com/kata-containers/kata-containers/releases/latest \
  -o /tmp/kata-static.tgz   # prendre le .tgz correspondant à l'arch
# ou paquet distro : sudo apt install kata-static
```

## Configuration containerd

`/etc/containerd/config.toml` :
```toml
[plugins."io.containerd.grpc.v1.cri".containerd.runtimes.kata-clh]
  runtime_type = "io.containerd.kata-v2"
  pod_annotations = []   # rien d'ouvert par défaut
```

Avec Docker : `docker run --runtime=kata-clh <image>` (Docker doit pointer sur le shim kata-v2 de containerd, ou utiliser le runtime docker-oci adapté).

## Configuration Kata — points de durcissement

Fichier : `/opt/kata/share/defaults/kata-containers/configuration-clh`

1. **Versions minimum** (corrections des CVE 2025-26 du doc 02) :
   - Kata ≥ **4.0.0** (ou ≥ 3.31.0 en 3.x) : corrige CVE-2026-50540 (config_path), CVE-2026-47243, CVE-2026-44210
2. **virtio-fs** (le maillon faible identifié) :
   - Ne jamais exposer le rootfs hôte. Restriction du source :
     ```
     virtio_fs_extra_args = "-o source=/var/lib/kata-shares --no-announce-submounts"
     ```
   - Jamais `--sandbox=none` (c'est ce qui a permis l'évasion CVE-2026-47243)
3. **Annotations** : désactiver/restrictifier `io.katacontainers.config_path` et `sandbox_bind_mounts` — ne jamais laisser un utilisateur de pod choisir le fichier de config du runtime (pattern CVE-2026-50540 / GHSA-7fhf)
4. **Agent guest** : seccomp guest actif (`disable_guest_seccomp = false` par défaut — le garder)
5. **Resources par sandbox** : plafonner vCPUs/RAM par défaut (ex. 2 vCPU / 2 Go) pour limiter le blast radius d'une DoS

## Hardening du kernel hôte (indispensable — c'est là que fuient les vrais leaks 2026)

`/etc/sysctl.d/99-sandbox-hardening.conf` :
```
# io_uring : vecteur PinTheft (CVE-2026-43494) + CVE-2025-39698/40364
# 2 = interdit même avec CAP_SYS_ADMIN ; 1 = seulement non-privilégié
# Essayer 2 d'abord ; si un service hôte (containerd daemon) en dépend, redescendre à 1
kernel.io_uring_disabled = 2

# user namespaces non privilégiés : clé de la plupart des LPE conteneur→hôte
user.max_user_namespaces = 0
# (Debian/Ubuntu : kernel.unprivileged_userns_clone = 0)

# core_pattern verrouillé (anti-redirect procfs, pattern CVE-2025-52881)
kernel.core_pattern = |/usr/share/systemd/systemd-coredump %P %u %g %s %t %c %h
```

En plus :
- **AppArmor/SELinux** : profil confiné pour le daemon containerd et le shim kata (le shim tourne root — c'est la cible de plusieurs évasions, cf. GHSA Dragonball)
- **Landlock** : actif côté hôte (mitigation des OOB sur les disques virtuels, pattern CVE-2026-27211)
- **Réseau** : chaque sandbox sur sa propre ns réseau, NAT egress, pas de port hôte exposé
- **Mise à jour** : unattended-upgrades sur kernel + kata + runtime ; abonnement aux feeds CVE Red Hat/Canonical

## GPU plus tard (roadmap)

- Passthrough VFIO de l'A100 (ou d'une partition) via cloud-hypervisor device hotplug — Kata le supporte
- **À valider sur le matériel cible avant de s'y engager** (IOMMU groups, configuration IOMMU du BIOS, version Kata supportant le hotplug PCI)
- Alternative si le passthrough entier est trop rigide : MIG (si A100) pour des slices 10 Go/20 Go

## Plan de validation (sur une VM de test d'abord)

1. Installer la stack sur une VM cloud de test (ou une machine de test) — vérifier `/dev/kvm`
2. `docker run --runtime=kata-clh alpine:latest` → vérifier : boot < 1 s, `ls /dev` minimal, pas de device hôte visible
3. **Tests d'évasion** avec les PoC publics du doc 02 (runc, virtio-fs) → tout doit être bloqué
4. Stress : 20 sandboxes parallèles → latence, mémoire réelle (objectif : < 200 Mo/sandbox au repos)
5. Rollout sur le serveur cible + monitoring (alerte sur les updates kata/kernel)

## Acceptance criteria

- [ ] Boot microVM < 1 s
- [ ] PoC d'évasion publics : 0 bypass
- [ ] 20 sandboxes simultanées stables
- [ ] Kernel hôte : io_uring/userns/core_pattern durcis vérifiés par `sysctl`
- [ ] GPU : PoC VFIO validé (phase 2)
