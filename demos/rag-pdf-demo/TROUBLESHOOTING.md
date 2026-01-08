# VelesDB RAG Demo - Quick Troubleshooting Guide

## 🚨 Common Connection Issues

### "ERR_CONNECTION_REFUSED" on localhost:8000?

**Solution immédiate :**
```
http://127.0.0.1:8000  # ✅ Fonctionne TOUJOURS
```

### Pourquoi localhost ne fonctionne pas ?

- **Windows Firewall** bloque parfois `localhost`
- **Antivirus** interfère avec la résolution DNS
- **VPN d'entreprise** redirige le trafic
- **DNS resolution** de "localhost" défaillante

### Commandes de diagnostic

```bash
# Vérifier si les serveurs tournent
netstat -ano | findstr ":8000"  # FastAPI Demo
netstat -ano | findstr ":8080"  # VelesDB Server

# Tester les endpoints
curl http://127.0.0.1:8000/health
curl http://127.0.0.1:8080

# Lister les processus
tasklist | findstr "18076"  # VelesDB PID
tasklist | findstr "17000"  # FastAPI PID
```

### URLs alternatives

- `http://127.0.0.1:8000/docs` - Documentation Swagger
- `http://127.0.0.1:8000/health` - Health check
- `http://127.0.0.1:8000/` - Interface principale

### Démarrage rapide

```bash
# Terminal 1: VelesDB Server
.\target\release\velesdb-server.exe --data-dir ./rag-data

# Terminal 2: FastAPI Demo
cd demos\rag-pdf-demo
uvicorn src.main:app --reload --port 8000

# Navigateur: utiliser 127.0.0.1 !
start http://127.0.0.1:8000
```

---

**Règle d'or :** Si `localhost` ne marche pas, utilisez `127.0.0.1`
