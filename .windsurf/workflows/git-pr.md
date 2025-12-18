---
description: Créer et gérer une Pull Request GitHub
---

# Workflow : Pull Request

## 1. Avant de créer la PR

1. S'assurer que la branche est à jour :
```powershell
git checkout develop
git pull origin develop
git checkout feature/ma-feature
git rebase develop
```

2. Résoudre les conflits si nécessaire

3. Vérifier la qualité :
// turbo
```powershell
cargo make ci
```

---

## 2. Pousser la branche

```powershell
git push -u origin feature/ma-feature
```

Si rebase, forcer le push :
```powershell
git push --force-with-lease origin feature/ma-feature
```

---

## 3. Créer la PR sur GitHub

1. Aller sur GitHub → Pull Requests → New

2. **Base** : `develop` (ou `main` pour hotfix)

3. **Compare** : `feature/ma-feature`

4. **Titre** : Suivre la convention commits
   ```
   feat(search): add hybrid search with BM25
   ```

5. **Description** : Utiliser le template
   ```markdown
   ## Description
   Brève description de ce que fait cette PR.

   ## Changes
   - Ajout de X
   - Modification de Y
   - Suppression de Z

   ## Testing
   - [ ] Tests unitaires ajoutés
   - [ ] Tests passent localement
   - [ ] Pas de régression

   ## Related Issues
   Closes #123
   ```

---

## 4. Review checklist

Avant de demander une review :

- [ ] Code formaté (`cargo fmt`)
- [ ] Pas de warnings clippy
- [ ] Tests ajoutés/mis à jour
- [ ] Documentation mise à jour
- [ ] CHANGELOG mis à jour si applicable
- [ ] Pas de secrets/credentials

---

## 5. Après merge

1. Supprimer la branche distante (GitHub le propose)

2. Localement :
```powershell
git checkout develop
git pull origin develop
git branch -d feature/ma-feature
```

3. Nettoyer les branches obsolètes :
```powershell
git fetch --prune
```
