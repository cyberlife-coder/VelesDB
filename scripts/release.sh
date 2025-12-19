#!/bin/bash
# =============================================================================
# VelesDB Core - Release Script
# =============================================================================
# Ce script automatise le processus de publication d'une nouvelle version
# de VelesDB-Core en utilisant le Versioning Sémantique (SemVer).
#
# Usage:
#   ./scripts/release.sh <version_type>
#   ./scripts/release.sh patch   # 0.1.0 -> 0.1.1
#   ./scripts/release.sh minor   # 0.1.0 -> 0.2.0
#   ./scripts/release.sh major   # 0.1.0 -> 1.0.0
#   ./scripts/release.sh 1.2.3   # Version explicite
# =============================================================================

set -e

# Couleurs pour les messages
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Fonctions utilitaires
info() { echo -e "${BLUE}ℹ️  $1${NC}"; }
success() { echo -e "${GREEN}✅ $1${NC}"; }
warning() { echo -e "${YELLOW}⚠️  $1${NC}"; }
error() { echo -e "${RED}❌ $1${NC}"; exit 1; }

# Vérification des prérequis
check_prerequisites() {
    info "Vérification des prérequis..."
    
    # Vérifier que nous sommes sur la branche main
    CURRENT_BRANCH=$(git branch --show-current)
    if [ "$CURRENT_BRANCH" != "main" ]; then
        error "Vous devez être sur la branche 'main' pour créer une release. Branche actuelle: $CURRENT_BRANCH"
    fi
    
    # Vérifier qu'il n'y a pas de changements non commités
    if [ -n "$(git status --porcelain)" ]; then
        error "Il y a des changements non commités. Veuillez les commiter ou les stasher avant de créer une release."
    fi
    
    # Vérifier que la branche est à jour
    git fetch origin main
    LOCAL=$(git rev-parse HEAD)
    REMOTE=$(git rev-parse origin/main)
    if [ "$LOCAL" != "$REMOTE" ]; then
        error "La branche locale n'est pas à jour avec origin/main. Veuillez faire un 'git pull'."
    fi
    
    success "Tous les prérequis sont satisfaits."
}

# Obtenir la version actuelle
get_current_version() {
    grep -m1 'version = ' Cargo.toml | sed 's/.*"\(.*\)".*/\1/'
}

# Calculer la nouvelle version
calculate_new_version() {
    local current_version=$1
    local version_type=$2
    
    IFS='.' read -r major minor patch <<< "$current_version"
    
    case $version_type in
        major)
            echo "$((major + 1)).0.0"
            ;;
        minor)
            echo "$major.$((minor + 1)).0"
            ;;
        patch)
            echo "$major.$minor.$((patch + 1))"
            ;;
        *)
            # Version explicite
            if [[ $version_type =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
                echo "$version_type"
            else
                error "Type de version invalide: $version_type. Utilisez 'major', 'minor', 'patch' ou une version explicite (ex: 1.2.3)."
            fi
            ;;
    esac
}

# Mettre à jour les fichiers Cargo.toml
update_cargo_versions() {
    local new_version=$1
    
    info "Mise à jour des versions dans Cargo.toml..."
    
    # Mettre à jour la version dans le workspace
    sed -i "s/^version = \".*\"/version = \"$new_version\"/" Cargo.toml
    
    # Mettre à jour les versions dans les crates
    for cargo_file in crates/*/Cargo.toml; do
        if [ -f "$cargo_file" ]; then
            sed -i "s/^version = \".*\"/version = \"$new_version\"/" "$cargo_file"
        fi
    done
    
    success "Versions mises à jour vers $new_version"
}

# Exécuter les tests
run_tests() {
    info "Exécution des tests..."
    
    cargo test --all-features || error "Les tests ont échoué. Corrigez les erreurs avant de créer une release."
    
    success "Tous les tests passent."
}

# Créer le commit et le tag
create_release_commit() {
    local new_version=$1
    
    info "Création du commit de release..."
    
    git add .
    git commit -m "chore(release): v$new_version"
    
    info "Création du tag v$new_version..."
    git tag -a "v$new_version" -m "Release v$new_version"
    
    success "Commit et tag créés."
}

# Pousser les changements
push_release() {
    local new_version=$1
    
    info "Push des changements vers origin..."
    
    git push origin main
    git push origin "v$new_version"
    
    success "Release v$new_version poussée vers origin."
}

# Script principal
main() {
    if [ -z "$1" ]; then
        echo "Usage: $0 <version_type>"
        echo "  version_type: major, minor, patch, ou une version explicite (ex: 1.2.3)"
        exit 1
    fi
    
    local version_type=$1
    
    echo ""
    echo "╔════════════════════════════════════════════════════════════════╗"
    echo "║           VelesDB Core - Release Script                        ║"
    echo "╚════════════════════════════════════════════════════════════════╝"
    echo ""
    
    check_prerequisites
    
    local current_version=$(get_current_version)
    local new_version=$(calculate_new_version "$current_version" "$version_type")
    
    info "Version actuelle: $current_version"
    info "Nouvelle version: $new_version"
    echo ""
    
    read -p "Voulez-vous continuer avec la release v$new_version ? (y/n) " -n 1 -r
    echo ""
    
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        warning "Release annulée."
        exit 0
    fi
    
    update_cargo_versions "$new_version"
    run_tests
    create_release_commit "$new_version"
    push_release "$new_version"
    
    echo ""
    echo "╔════════════════════════════════════════════════════════════════╗"
    echo "║                    🎉 Release Complète !                       ║"
    echo "╚════════════════════════════════════════════════════════════════╝"
    echo ""
    success "VelesDB Core v$new_version a été publiée avec succès !"
    info "Le workflow GitHub Actions va maintenant créer les binaires et publier sur crates.io."
    echo ""
}

main "$@"
