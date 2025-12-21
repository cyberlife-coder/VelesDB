#!/bin/bash
# =============================================================================
# VelesDB Installer Script
# =============================================================================
# One-liner installation for Linux and macOS:
#   curl -fsSL https://raw.githubusercontent.com/cyberlife-coder/VelesDB/main/scripts/install.sh | bash
# =============================================================================

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
REPO="cyberlife-coder/VelesDB"
INSTALL_DIR="${VELESDB_INSTALL_DIR:-$HOME/.velesdb}"
BIN_DIR="${VELESDB_BIN_DIR:-$HOME/.local/bin}"

echo -e "${BLUE}"
echo "╔═══════════════════════════════════════════════════════════════╗"
echo "║                                                               ║"
echo "║   🐺 VelesDB Installer                                        ║"
echo "║   High-Performance Vector Database for AI Applications        ║"
echo "║                                                               ║"
echo "╚═══════════════════════════════════════════════════════════════╝"
echo -e "${NC}"

# Detect OS and architecture
detect_platform() {
    local os arch
    
    case "$(uname -s)" in
        Linux*)  os="linux" ;;
        Darwin*) os="macos" ;;
        *)       echo -e "${RED}❌ Unsupported OS: $(uname -s)${NC}"; exit 1 ;;
    esac
    
    case "$(uname -m)" in
        x86_64)  arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *)       echo -e "${RED}❌ Unsupported architecture: $(uname -m)${NC}"; exit 1 ;;
    esac
    
    echo "${os}-${arch}"
}

# Get latest release version
get_latest_version() {
    curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | \
        grep '"tag_name":' | sed -E 's/.*"v([^"]+)".*/\1/'
}

# Download and install
install_velesdb() {
    local platform version download_url archive_name
    
    platform=$(detect_platform)
    echo -e "${YELLOW}📍 Detected platform: ${platform}${NC}"
    
    # Get latest version
    echo -e "${YELLOW}🔍 Fetching latest version...${NC}"
    version=$(get_latest_version)
    
    if [ -z "$version" ]; then
        echo -e "${RED}❌ Failed to get latest version${NC}"
        exit 1
    fi
    
    echo -e "${GREEN}📦 Latest version: v${version}${NC}"
    
    # Construct download URL
    case "$platform" in
        linux-x86_64)  archive_name="velesdb-linux-x86_64.tar.gz" ;;
        macos-x86_64)  archive_name="velesdb-macos-x86_64.tar.gz" ;;
        macos-aarch64) archive_name="velesdb-macos-aarch64.tar.gz" ;;
        *)             echo -e "${RED}❌ No binary available for ${platform}${NC}"; exit 1 ;;
    esac
    
    download_url="https://github.com/${REPO}/releases/download/v${version}/${archive_name}"
    
    # Create directories
    echo -e "${YELLOW}📁 Creating directories...${NC}"
    mkdir -p "$INSTALL_DIR"
    mkdir -p "$BIN_DIR"
    
    # Download
    echo -e "${YELLOW}⬇️  Downloading VelesDB v${version}...${NC}"
    local tmp_dir=$(mktemp -d)
    curl -fsSL "$download_url" -o "${tmp_dir}/${archive_name}"
    
    # Extract
    echo -e "${YELLOW}📦 Extracting...${NC}"
    tar -xzf "${tmp_dir}/${archive_name}" -C "$INSTALL_DIR"
    
    # Create symlinks
    echo -e "${YELLOW}🔗 Creating symlinks...${NC}"
    ln -sf "$INSTALL_DIR/velesdb" "$BIN_DIR/velesdb"
    ln -sf "$INSTALL_DIR/velesdb-server" "$BIN_DIR/velesdb-server"
    
    # Cleanup
    rm -rf "$tmp_dir"
    
    # Verify installation
    if [ -x "$BIN_DIR/velesdb" ]; then
        echo -e "${GREEN}✅ VelesDB installed successfully!${NC}"
    else
        echo -e "${RED}❌ Installation failed${NC}"
        exit 1
    fi
}

# Add to PATH instructions
print_path_instructions() {
    local shell_config
    
    case "$SHELL" in
        */bash) shell_config="$HOME/.bashrc" ;;
        */zsh)  shell_config="$HOME/.zshrc" ;;
        *)      shell_config="your shell config" ;;
    esac
    
    if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
        echo ""
        echo -e "${YELLOW}📝 Add to your PATH by running:${NC}"
        echo ""
        echo -e "   echo 'export PATH=\"\$PATH:$BIN_DIR\"' >> $shell_config"
        echo -e "   source $shell_config"
        echo ""
    fi
}

# Print usage
print_usage() {
    echo ""
    echo -e "${BLUE}🚀 Quick Start:${NC}"
    echo ""
    echo "   # Start the server"
    echo "   velesdb-server"
    echo ""
    echo "   # Open interactive CLI"
    echo "   velesdb"
    echo ""
    echo "   # Create a collection"
    echo "   curl -X POST http://localhost:8080/collections \\"
    echo "     -d '{\"name\": \"docs\", \"dimension\": 768, \"metric\": \"cosine\"}'"
    echo ""
    echo -e "${BLUE}📚 Documentation:${NC} https://github.com/${REPO}#readme"
    echo ""
}

# Main
main() {
    install_velesdb
    print_path_instructions
    print_usage
}

main "$@"
