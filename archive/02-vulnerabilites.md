# Historique de vulnérabilités 2021–2026 & indice de confiance

Document de travail — v2, 2026-10-17. Complète `01-comparatif.md`.

**Objectif** : alimenter la section sécurité de la spécification de l'orchestrateur. Menace de référence : workloads non fiables (code d'agents, builds, jobs) sur des hôtes partagés ; adversaire = workload compromis.

## Méthode

- **Fenêtre** : janvier 2021 – octobre 2026
- **Sources** : API NVD 2.0, CISA KEV (catalogue complet vérifié le 2026-10-17, 1 748 entrées), advisories GitHub (runc, gvisor, kata-containers, firecracker, cloud-hypervisor, rust-vmm), advisories éditeurs (Red Hat, AWS, VMware, Microsoft), blogs sécurité (OtterSec, Edera, Wiz, Datadog, CrowdStrike, Huntress, CloudLinux)
- **Filtre** : fuites guest/conteneur→hôte, LPE atteignables depuis un conteneur, RCE hyperviseur. Les CVE mineures sans vecteur d'évasion sont exclues
- **Aucune CVE inventée** : les ID non vérifiables (dont 3 CVE VMware initialement proposées) ont été écartés

## Récapitulatif par catégorie

### Cat 1 — Kernel partagé (primitives process / vecteurs conteneurs)

~20 CVE avec vecteur conteneur sur la fenêtre ; **5+ exploitées in the wild**.

| CVE | Année | Composant | CVSS | ITW | Impact |
|-----|-------|-----------|------|-----|--------|
| CVE-2021-22555 | 2021 | netfilter | 8.3 | ✅ KEV 2025 | LPE depuis userns → fuite |
| CVE-2022-0185 | 2022 | fs_context « Leaky VFS » | 8.4 | ✅ KEV | Fuite conteneur→hôte K8s |
| CVE-2022-0492 | 2022 | cgroups v1 release_agent | 7.8 | — | Root hôte depuis conteneur |
| CVE-2022-0847 | 2022 | pipe « Dirty Pipe » | 7.8 | ✅ KEV | Écriture arbitraire → LPE/fuite (PoC Datadog) |
| CVE-2023-0386 | 2023 | overlayfs setuid | 7.8 | ✅ KEV 2025 | LPE ; overlayfs = rootfs Docker |
| CVE-2023-2640 + 32629 | 2023 | overlayfs « GameOver(lay) » | 7.8 | — | Rooting de conteneurs non-root (Wiz/CrowdStrike) |
| CVE-2023-32233 | 2023 | act_mirred | 7.8 | — | UAF → LPE depuis userns |
| CVE-2023-4244 | 2023 | nf_tables | 7.8 | — | UAF → LPE |
| CVE-2024-1086 (+1085) | 2024 | nf_tables « Flipping Pages » | 7.8 | ✅ KEV, ransomware | LPE ; sur kernel partagé = compromission hôte/voisins |
| CVE-2024-53106 | 2024 | IMA (LSM) | 7.3 | — | LPE |
| CVE-2025-39698 / 40364 | 2025 | io_uring | 7.8 | — | LPE ; bloqué par seccomp Docker par défaut |
| CVE-2026-31431 | 2026 | AF_ALG « CopyFail » | 7.8 | — | LPE (précurseur Dirty Frag) |
| CVE-2026-43284 (+43500) | 2026 | xfrm-ESP/RxRPC « Dirty Frag » | 8.8 | ✅ attaques actives (MS, mai 2026) | LPE déterministe + fuite conteneur |
| CVE-2026-43494 | 2026 | rds+io_uring « PinTheft » | 7.8 | — | LPE |
| CVE-2026-43499 | 2026 | rtmutex/futex « GhostLock » | 7.8 | — | Fuite conteneur→hôte confirmée (kernelCTF) |
| CVE-2026-46300 | 2026 | skbuff/xfrm « Fragnesia » | 7.8 | — | LPE + fuite conteneur |
| CVE-2026-80521 | 2026 | AF_UNIX | 7.8 | — | Fuite conteneur→root hôte (PoC public) |

Clusters : **nf_tables** (5 CVE, 2 ITW), **overlayfs** (Docker rootfs), **vague « page-cache » 2026** (6 CVE). User namespaces = clé qui rend ces LPE atteignables depuis un conteneur non privilégié.

### Cat 2 — Conteneurs userspace

| CVE | Année | Composant | CVSS | ITW | Impact |
|-----|-------|-----------|------|-----|--------|
| CVE-2021-30465 | 2021 | runc (symlink) | 8.5 | — | Fuite conteneur→hôte |
| CVE-2024-21626 | 2024 | runc « Leaky Vessels » | 8.6 | — | Fuite complète (fd /sys/fs/cgroup hôte) |
| CVE-2024-23651/23652/23653 | 2024 | BuildKit/Docker build | 8.7–10.0 | — | 3 fuites build-time→hôte (dont CVSS 10) |
| CVE-2024-45310 | 2024 | runc (O_TRUNC) | 3.6 | — | Gadget d'écriture hôte (réutilisé nov. 2025) |
| CVE-2025-31133 | 2025 | runc (masked path) | 7.8 | — | Fuite/DoS hôte |
| CVE-2025-52565 | 2025 | runc (/dev/console) | 7.5 | — | Fuite/DoS |
| CVE-2025-52881 | 2025 | runc (procfs core_pattern) | 7.5 | — | Fuite + bypass de labels LSM |

**Nov. 2025 : 3 breakouts runc en une salve** (CNCF advisory). Pas de CVE spécifique containerd / LXC / LXD / Podman trouvées — mais Podman/crun exposés via les failles runc.

### Cat 3 — Faux kernel

**gVisor (runsc)** — 7 CVE sur la fenêtre, profil remarquable :
| CVE | Année | CVSS | Impact |
|-----|-------|------|--------|
| CVE-2023-7258 | 2023 | 4.8 | DoS du sandbox (pas d'évasion) |
| CVE-2024-10026 / 10603 | 2024 | 5.3 | Fingerprinting réseau (travail NDSS 2025) |
| CVE-2025-2713 | 2025 | 7.8 | LPE interne à runsc |
| **CVE-2026-96812** | 2026 | 8.8 | **1re évasion hôte** (CUSE) — nécessite config CUSE + image malveillante |
| CVE-2026-95702 | 2026 | 8.5 | UAF sentry (confiné par seccomp) |
| CVE-2026-107935 | 2026 | 9.3 | gvproxy (écosystème, pas le sentry) |

**Zéro évasion hôte de 2018 à 2025.** Politique CVE stricte : CVE uniquement si le bug franchit la frontière ET est gVisor-specific.

**Kata Containers** — trou 2021–2024 (0 CVE), puis **salve de 9 CVE + 2 GHSA non cotées en 16 mois** (sept. 2025–août 2026), dont de vraies évasions guest→hôte :
| CVE | Année | CVSS | Impact |
|-----|-------|------|--------|
| CVE-2025-58354 | 2025 | 6.9 | CoCo/TDX : hôte malveillant contourne l'attestation |
| CVE-2026-24054 | 2026 | 8.8 | Image malformée → hotplug de device hôte dans la microVM |
| CVE-2026-24834 | 2026 | 9.3 | pmem/DAX → RCE root DANS le guest (pas d'évasion hôte) |
| CVE-2026-41326 | 2026 | 8.2 | CoCo : hôte non fiable écrit dans l'image guest |
| **CVE-2026-44210** | 2026 | 9.9 (Red Hat) | Injection virtiofsd via annotation → mount rootfs hôte, /etc/shadow |
| **CVE-2026-50540** | 2026 | 9.6 | Annotation config_path → TOML hôte arbitraire → RCE root hôte |
| **CVE-2026-47243** | 2026 | 9.2 | virtio-fs standalone → symlinks root /etc/cron.d → RCE hôte |
| GHSA-7fhf-v3p3-rp56 | 2026 | high | Annotation sandbox_bind_mounts (trouvée par agents IA/Anthropic) |
| GHSA-fgm4-mv68-h344 | 2026 | high | Dragonball virtio-blk : OOB via io_uring → **RCE root nœud** (évasion) |

Pattern : concentration sur **virtiofsd, annotations de pod, CoCo/TEE** — le runtime userspace et la config, pas le cœur de la microVM.

**libkrun** : **0 CVE directe** (aucun advisory GitHub, aucune CPE NVD ; seul adjacent : CVE-2026-84042 côté crun). À interpréter comme **non mesuré**, pas comme sûr.

### Cat 4 — MicroVMs

Volume très faible ; 2021–2025 quasi vide, 4 des 6 CVE datées 2026. **Aucune au CISA KEV, aucune ITW.**
| CVE | Année | Composant | CVSS | Impact |
|-----|-------|-----------|------|--------|
| CVE-2023-30612 | 2023 | Cloud Hypervisor (API) | 4.9 | DoS local |
| CVE-2023-50711 | 2024 | rust-vmm | 5.7 | UB/DoS dans le VMM |
| CVE-2026-1386 | 2026 | Firecracker (jailer) | 6.0 | LPE local hôte (symlink) |
| **CVE-2026-5747** | 2026 | Firecracker (virtio-PCI) | 8.7 | Guest root → OOB 524 Ko → vecteur d'évasion (corrigé 1.14.4/1.15.1) |
| **CVE-2026-27211** | 2026 | Cloud Hypervisor (virtio-block) | 9.1 | **Exfiltration de fichiers hôte** via fausse image QCOW2 |
| **CVE-2026-45782** | 2026 | Cloud Hypervisor (virtio-block) | 8.9 | UAF → corruption VMM (vecteur d'évasion) + fuite heap |

Hors fenêtre : CVE-2019-18960 (Firecracker vsock, 9.8), CVE-2020-2025 (Kata/CH guest→host 8.8) — la surface existait déjà. stratoVirt : 0 CVE publique.

**Docker Sandbox (sbx)** — produit dédié cat 4 (microVM avec VMM propriétaire Docker sur hyperviseur natif : KVM / Apple Hypervisor.framework / Windows Hypervisor Platform ; l'agent tourne en conteneur dans la VM = cat 2 imbriquée dans cat 4, frontière effective = hyperviseur) :
| CVE | Année | Composant | Impact |
|-----|-------|-----------|--------|
| CVE-2026-18171 | 2026 | virtio-fs (policy-share) | Intention read-only non appliquée au bord hôte → écriture dans des répertoires hôte « read-only » (CWE-863) |
| CVE-2026-79994 | 2026 | UDS forwarder | Symlink → le guest atteint les sockets Unix de l'hôte (fuite guest→hôte malgré la microVM) |
| CVE-2026-77179 | 2026 | macOS | Échappement du sandbox |

Indice de confiance sbx : **mécanisme B+** (frontière hyperviseur solide, mais VMM jeune 2025, audité bien moins que Firecracker/Cloud Hypervisor) ; **produit B-** (3 CVE dès 2026 — toutes sur des composants userspace, pattern identique au constat global).

### Cat 5 — VMs pleines

| CVE | Année | Produit | CVSS | ITW | Impact |
|-----|-------|---------|------|-----|--------|
| CVE-2021-3748 | 2021 | QEMU virtio-net | 7.5 | — | UAF → crash/RCE hôte |
| CVE-2022-30163 | 2022 | Hyper-V | 8.5 | — | RCE hyperviseur guest→host |
| CVE-2023-46842 | 2023 | Xen | 6.5 | — | DoS hyperviseur |
| CVE-2023-34048 | 2023 | vCenter | 9.8 | ✅ KEV | RCE non authentifié |
| CVE-2024-21407 | 2024 | Hyper-V | 8.1 | — | **Échappement de VM documenté** |
| CVE-2024-22273 | 2024 | ESXi | 8.1 | — | OOB stockage → compromission VMX |
| CVE-2024-37079 | 2024 | vCenter | 9.8 | ✅ KEV 2026 | RCE non authentifié |
| CVE-2024-37085 | 2024 | ESXi (AD) | 6.8 | ✅ KEV, ransomware (STORM-1175) | Bypass d'auth → admin hôte |
| CVE-2024-49117 | 2024 | Hyper-V | 8.8 | — | RCE guest→host (droits réduits) |
| CVE-2024-7730 | 2024 | QEMU virtio-snd | 7.4 | — | **Échappement guest→host public** (OtterSec) |
| CVE-2025-22224/22225/22226 | 2025 | ESXi « ESXicape » | 9.3 | ✅ KEV, ITW (kit 155 builds) | Admin VM → code VMX → échappement hôte |
| CVE-2026-3195 | 2026 | QEMU virtio-snd | 7.4 | — | **Correction incomplète de CVE-2024-7730** |
| CVE-2026-62428 / 62435 | 2026 | Xen (XSA-500/501) | 7.8 / 6.5 | — | Grant tables : escalade guest→hôte |

Contexte adjacent (affecte tous les hôtes KVM, donc cats 3–5) : **CVE-2026-99999 « PwnEscape »** (9.3, oct. 2026) — évasion KVM guest→hôte via ipv4 checksum, reproduite indépendamment.

## Timeline des faits marquants

- **2021** : netfilter (22555), runc symlink (30465), Hyper-V host RCE, QEMU virtio-net
- **2022** : Dirty Pipe + Leaky VFS (KEV), cgroups v1 — l'année où « les conteneurs fuient » entre dans le vocabulaire courant
- **2023** : GameOver(lay), nf_tables en série, gVisor DoS
- **2024** : Flipping Pages (ransomware), Leaky Vessels (runc+BuildKit), ESXi AD ransomware, vCenter KEV, 1er échappement QEMU public complet, Hyper-V VM escape
- **2025** : ESXicape (ITW, KEV), runc 3 breakouts (nov.), io_uring, gVisor LPE interne
- **2026** : **année des salves** — gVisor 1re évasion (oct.), Kata 9+2 en 16 mois, microVMs 4 CVE, QEMU correction incomplète, Xen grant tables, KVM PwnEscape, vague Linux « page-cache » (Dirty Frag ITW…)

## Indice de confiance

Avertissement épistémologique : le **nombre de CVE publiées n'est pas une mesure directe du risque**. QEMU est le plus « vulnérable » sur le papier mais aussi le plus audité et le plus déployé. La faible densité chez Firecracker/libkrun reflète aussi un audit public plus jeune.

4 critères : (a) indépendance structurelle de la frontière, (b) fuites observées sur la fenêtre, (c) exploitation ITW, (d) discipline de patching/transparence.

| Cat | Frontière | Fuites observées (2021–26) | ITW | Mécanisme | Produit/écosystème |
|-----|-----------|---------------------------|-----|-----------|--------------------|
| 1 | kernel partagé (aucune) | ~15 LPE à vecteur conteneur | ✅ x5 | **D** | C+ (patching kernel rapide) |
| 2 | kernel + runtime userspace | 6 runc + 3 BuildKit | ✅ (côté kernel) | **C-** | B+ |
| 3 | userspace kernel / microVM | gVisor 1 (config) ; Kata 4–5 (virtiofsd/annotations) | ❌ | **B+** (gVisor A-) | B- (salve Kata 2026) |
| 4 | microVM (device model minimal) | 3 vecteurs (2026, non exploités) | ❌ | **A-** | B+ (petit écosystème, patching rapide) |
| 5 | VT-x matériel | nombreuses (QEMU/Xen/VMware/Hyper-V) | ✅ (chaînes VMware, vCenter) | **A-** (VT-x intouché, évasions via device model/userspace) | B- (QEMU volume élevé, VMware cible ITW) |

## Synthèse — ce que l'historique dit vraiment

1. **Le kernel Linux hôte est la surface dominante** — il traverse TOUTES les catégories (même microVMs/VMs via KVM, voir PwnEscape). Aucune technologie d'isolation n'échappe au kernel hôte.
2. **Le maillon faible est rarement le mécanisme de virtualisation, c'est le runtime userspace** : runc (6 fuites), virtiofsd + annotations (Kata), jailer (Firecracker), CUSE (gVisor). Les évasions « pures » (VT-x, KVM) sont rarissimes.
3. **La configuration fait la sécurité** : CUSE activé (gVisor), annotations Kata non restreintes, virtio-fs standalone, images non fiables (QCOW2) — la plupart des fuites 2026 sont des échecs de configuration, pas des breaks fondamentaux.
4. **Zéro CVE ≠ sûr** : libkrun et stratoVirt n'ont pas de publication — absence de mesure ≠ solidité démontrée (arXiv 2606.08433, §3.5 : « unmeasured ≠ safe »).
5. **La fréquence VMware = exposition, pas mécanisme faible** : vCenter/ESXi sont exposés sur Internet et ciblés par ransomware ; le VT-x lui-même n'a jamais été breaké.
6. **2026 = maturation** : les technologies jeunes (Kata, microVMs) commencent à produire des CVE — signal de plus d'audits et de recherche, pas nécessairement de régression. gVisor après 8 ans sans évasion est le cas le plus parlant.

## Implications pour la spec de l'orchestrateur

1. **Patch SLA** : CVE kernel avec vecteur conteneur/VM → patch < 72 h sur tous les hôtes (la vague 2026 « page-cache » a montré la vitesse d'exploitation)
2. **Cible d'audit n°1** : les runtimes userspace (virtio-fs, forwarders, shims root) — les breaks du mécanisme de virtualisation sont rarissimes
3. **Default-deny** : parts host, annotations, config_path — jamais de chemin hôte arbitraire accessible depuis un workload
4. **Hardening kernel systématique** sur tout hôte du pool : io_uring, userns, core_pattern (cf. doc 03)

## Sources principales

- CISA KEV : https://www.cisa.gov/known-exploited-vulnerabilities-catalog (vérifié 2026-10-17)
- NVD 2.0 (chaque CVE vérifiée individuellement)
- OtterSec (QEMU escape) : https://osec.io/blog/virtio-snd-qemu-hypervisor-escape/
- Edera (Kata CVE-2026-24834) : https://edera.dev/stories/cve-2026-24834-when-trusting-the-guest-goes-wrong
- Wiz (Leaky Vessels, Dirty Frag) : https://www.wiz.io/blog
- Huntress (ESXicape) : https://www.huntress.com/blog/esxi-vm-escape-exploit
- Microsoft (Dirty Frag) : https://www.microsoft.com/en-us/security/blog/2026-05-08/
- CNCF (runc nov. 2025) : https://www.cncf.io/blog/2025-11-28/runc-container-breakout-vulnerabilities-a-technical-overview/
- arXiv 2606.08433 « AI Code Sandboxes: A Comparative Security Study »
