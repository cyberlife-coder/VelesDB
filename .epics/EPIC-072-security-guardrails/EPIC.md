# EPIC-072: Security & Guard Rails

## Status: TODO
## Priority: HAUTE
## Sprint: 2026-Q1

## Vision

Renforcer les protections contre les abus et les attaques DoS.

## Problème Actuel

- `velesdb-cli` bypass les `GuardRails` (pas de timeout par défaut)
- `CircuitBreaker` reset après restart → DoS possible
- Pas de rate limiting persistant

## User Stories

| US | Description | Priorité | Estimation |
|----|-------------|----------|------------|
| US-001 | Ajouter --timeout par défaut au CLI | HAUTE | 2h |
| US-002 | Persister état CircuitBreaker | MOYENNE | 4h |

## Fichiers Critiques

- `crates/velesdb-cli/src/main.rs`
- `guardrails.rs` - CircuitBreaker

## Critères d'Acceptation

- [ ] CLI a timeout par défaut (30s)
- [ ] CircuitBreaker état persisté dans fichier
- [ ] Tests de charge avec rate limiting
