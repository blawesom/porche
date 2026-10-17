# Technologies d'isolation — comparatif (catégories 1 à 5)

Document de travail Benjamin — v2, 2026-10-17.

**Objectif** : fondement technique de la spécification de l'orchestrateur de cloud maison — choix de la couche d'isolation pour workloads non fiables (agents IA, jobs, builds) et GPU.

## Catégories

1. **Isolation process** — primitives kernel (namespaces, cgroups, seccomp, LSM) + outils (bwrap, nsjail, firejail)
2. **Containers userspace** — runc, Docker, Podman, LXC/LXD, systemd-nspawn
3. **Sandbox « faux kernel »** — gVisor, libkrun (kernel userspace) ; Kata Containers (API container, isolation microVM)
4. **MicroVMs** — Firecracker, Cloud Hypervisor
5. **VMs pleines** — QEMU/KVM + libvirt, Xen, VMware, VirtualBox, Hyper-V, Proxmox VE

## Tableau de comparaison

| Cat | Tech | Isolation | Amorçage | Overhead runtime | Densité / node | GPU | Écosystème |
|-----|------|-----------|----------|------------------|----------------|-----|------------|
| 1 | namespaces/cgroups, seccomp, AppArmor/SELinux, bwrap, nsjail | Faible–moyenne : kernel hôte partagé. Correcte contre du code local non fiable si seccomp+LSM bien configurés | < 10 ms | ≈ 0 % | Très haute (milliers) | Oui | Minimal (primitives brutes) |
| 2 | runc, Docker, Podman (rootless), LXC/LXD, systemd-nspawn | Moyenne : kernel hôte partagé, frontière config-dépendante | 10–100 ms | ~1–5 % | Très haute | Oui (nvidia-container-toolkit) | Très large (OCI, Kubernetes, registres) |
| 3 | gVisor, libkrun, Kata Containers | Haute : surface d'attaque kernel réduite (3a) ou guest dédié (Kata) | 50–200 ms | 10–50 % sur workloads syscalls-intensifs | Haute | Kata/libkrun : oui ; gVisor : non | Moyen (K8s : GKE Sandbox, runwasi, kata-runtime) |
| 4 | Firecracker, Cloud Hypervisor | Haute : guest kernel dédié, modèle de devices minimal | ~125 ms (Firecracker) | Bas pour une VM (~5 %) | Haute (milliers de petites inst.) | Firecracker : non ; Cloud Hypervisor : VFIO | Moyen (AWS Lambda/Fargate, E2B, Fly.io Machines) |
| 5 | QEMU/KVM + libvirt, Xen, Proxmox, VMware, Hyper-V | Très haute : virtualisation matérielle VT-x, guest complet | 5–30 s | ~3–10 % | Faible (dizaines) | Oui (VFIO passthrough) | Très large (cloud, live migration, HA) |

## Usages recommandés

- **Cat 1** — Tâches CLI éphémères, sandbox de commandes non fiables exécutées localement (ex. exécution d'outils/agents IA), CI de pipelines de confiance. **Pas** pour du multi-tenant.
- **Cat 2** — Le choix par défaut : workloads apps standard, microservices, environnements de dev, Kubernetes. LXC/LXD : quand on veut un « système complet » (init, réseau dédié, mode unpriv).
- **Cat 3** — Serverless multi-tenant à haute densité, code utilisateur non fiable sur un host partagé (modèle GKE Sandbox). Compromis isolation/perf/densité.
- **Cat 4** — FaaS, sandbox de code non fiable à très haute densité (sandailes d'agents : E2B, Fly.io), CI jetable. Firecracker est LA référence.
- **Cat 5** — Passthrough GPU (ex. A100), OS complets hétérogènes (Windows, autres distros), live migration, infra critique où l'isolation prime sur la densité. Proxmox pour de la gestion on-prem (KVM+LXC, backups PBS, clustering).

## Critères de décision

1. **Qui exécute le code ?** — confiance / semi-confiance / non fiable → fixe le niveau d'isolation minimal.
2. **Densité et latence d'amorçage** — milliers de boîtes jetables (→ 1/2/4) vs dizaines de VMs long-running (→ 5).
3. **Accès GPU / hardware** — passthrough VFIO (→ 4 CH/5), toolkit conteneur (→ 2), exclu (gVisor pur).
4. **Écosystème requis** — OCI/images/K8s (→ 2/3), migration/HA (→ 5), API serverless (→ 4).

## Patterns hybrides notables

- **Kata Containers** = API container + microVM : la façon la plus simple d'avoir isolation VM sous Kubernetes sans réécrire l'orchestration.
- **LXD unpriv** = interface conteneur, isolation proche VM (user namespaces + KVM si dispo).
- **K8s + Firecracker** = base des plateformes FaaS multi-tenant (modèle Fly.io).

## À creuser ensuite

- [ ] Benchmarks concrets (amorçage, CPU, I/O) sur notre hardware
- [ ] Cas d'usage cible : exécution d'agents / workloads AI / dev env ?
- [ ] GPU : comment exposer l'A100 selon la tech retenue
