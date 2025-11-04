#!/bin/bash

set -e

# Default version
VERSION="${1:-v0.2.0}"
REPO="logos-co/nomos-circuits"
INSTALL_DIR="$HOME/.nomos-circuits"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

print_info() {
    echo -e "${BLUE}ℹ${NC} $1"
}

print_success() {
    echo -e "${GREEN}✓${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}⚠${NC} $1"
}

print_error() {
    echo -e "${RED}✗${NC} $1"
}

# Detect OS and architecture
detect_platform() {
    local os=""
    local arch=""

    # Detect OS
    case "$(uname -s)" in
        Linux*)     os="linux";;
        Darwin*)    os="macos";;
        MINGW*|MSYS*|CYGWIN*)    os="windows";;
        *)          print_error "Unsupported operating system: $(uname -s)"; exit 1;;
    esac

    # Detect architecture
    case "$(uname -m)" in
        x86_64)     arch="x86_64";;
        aarch64)    arch="aarch64";;
        arm64)      arch="aarch64";;
        *)          print_error "Unsupported architecture: $(uname -m)"; exit 1;;
    esac

    echo "${os}-${arch}"
}

# Check if installation directory exists and get confirmation
check_existing_installation() {
    if [ -d "$INSTALL_DIR" ]; then
        print_warning "Installation directory already exists: $INSTALL_DIR"

        # Check if it has a VERSION file
        if [ -f "$INSTALL_DIR/VERSION" ]; then
            local current_version=$(cat "$INSTALL_DIR/VERSION")
            print_info "Currently installed version: $current_version"
        fi

        # In non-interactive environments (CI), automatically overwrite
        if [ ! -t 0 ]; then
            print_info "Non-interactive environment detected, automatically overwriting..."
        else
            # Interactive environment - ask for confirmation
            echo
            read -p "Do you want to overwrite it? (y/N): " -n 1 -r
            echo
            if [[ ! $REPLY =~ ^[Yy]$ ]]; then
                print_info "Installation cancelled."
                exit 0
            fi
        fi

        print_info "Removing existing installation..."
        rm -rf "$INSTALL_DIR"
    fi
}

# Download and extract the release
download_release() {
    local platform="$1"
    local artifact="nomos-circuits-${VERSION}-${platform}.tar.gz"
    local url="https://github.com/${REPO}/releases/download/${VERSION}/${artifact}"
    local temp_dir=$(mktemp -d)

    print_info "Downloading nomos-circuits ${VERSION} for ${platform}..."
    print_info "URL: $url"

    if ! curl -L -o "${temp_dir}/${artifact}" "$url"; then
        print_error "Failed to download release artifact"
        print_error "Please check that version ${VERSION} exists for platform ${platform}"
        print_error "Available releases: https://github.com/${REPO}/releases"
        rm -rf "$temp_dir"
        exit 1
    fi

    print_success "Download complete"

    print_info "Extracting to ${INSTALL_DIR}..."
    mkdir -p "$INSTALL_DIR"

    if ! tar -xzf "${temp_dir}/${artifact}" -C "$INSTALL_DIR" --strip-components=1; then
        print_error "Failed to extract archive"
        rm -rf "$temp_dir"
        exit 1
    fi

    rm -rf "$temp_dir"
    print_success "Extraction complete"
}

# Handle macOS code signing/quarantine issues
handle_macos_quarantine() {
    # Skip authorization in non-interactive environments (CI)
    if [ ! -t 0 ]; then
        print_info "Non-interactive environment detected, skipping macOS authorization process"
        print_info "Binaries may need to be authorized manually if quarantined"
        return
    fi

    print_warning "macOS detected: Handling code signing requirements"
    echo
    print_info "The nomos-circuits binaries are not yet code-signed for macOS."
    print_info "You will need to authorize each binary to run."
    echo
    print_info "This process has TWO steps for each binary:"
    print_info "  1. First attempt will show a quarantine warning"
    print_info "  2. Go to System Settings > Privacy & Security"
    print_info "  3. Click 'Allow Anyway' for the blocked binary"
    print_info "  4. Second attempt will show a final confirmation dialog"
    print_info "  5. Click 'Open' to authorize the binary"
    echo

    read -p "Press Enter to begin the authorization process..."
    echo

    # Find all executables by name (prover, verifier, witness_generator)
    local executables=$(find "$INSTALL_DIR" -type f \( -name "prover" -o -name "verifier" -o -name "witness_generator" \) | sort)
    local current=0
    local total=$(echo "$executables" | wc -l)

    for binary in $executables; do
        current=$((current + 1))
        local binary_name=$(basename "$binary")
        local circuit_name=$(basename $(dirname "$binary"))

        if [ ! -f "$binary" ]; then
            print_warning "Binary not found: $binary (skipping)"
            continue
        fi

        echo
        print_info "[$current/$total] Authorizing: $binary_name"
        if [ "$circuit_name" != "." ] && [ "$circuit_name" != ".nomos-circuits" ]; then
            print_info "         (from $circuit_name circuit)"
        fi
        echo

        # First attempt - will trigger quarantine
        print_info "STEP 1: First attempt (will be blocked)..."
        "$binary" --help 2>/dev/null || true
        sleep 1

        echo
        print_warning "ACTION REQUIRED:"
        print_warning "  1. Open System Settings > Privacy & Security"
        print_warning "  2. Look for a message about '$binary_name'"
        print_warning "  3. Click 'Allow Anyway'"
        echo
        read -p "Press Enter after clicking 'Allow Anyway'..."

        # Second attempt - will show final confirmation
        print_info "STEP 2: Second attempt (click 'Open' in the dialog)..."
        sleep 1
        "$binary" --help >/dev/null 2>&1 || true

        print_success "Authorization process complete for $binary_name"
        sleep 1
    done

    echo
    print_success "All binaries processed!"
    print_info "If any binaries still don't work, you can authorize them manually by:"
    print_info "  1. Try running the binary: <binary> --help"
    print_info "  2. Go to System Settings > Privacy & Security"
    print_info "  3. Click 'Allow Anyway' and try again"
}

# Main installation process
main() {
    print_info "Setting up nomos-circuits ${VERSION}"
    echo

    # Detect platform
    local platform=$(detect_platform)
    print_info "Detected platform: $platform"

    # Check existing installation
    check_existing_installation

    # Download and extract
    download_release "$platform"

    # Handle macOS quarantine if needed
    if [[ "$platform" == macos-* ]]; then
        echo
        handle_macos_quarantine
    fi

    echo
    print_success "Installation complete!"
    echo
    print_info "nomos-circuits ${VERSION} is now installed at: $INSTALL_DIR"
    print_info "The following circuits are available:"

    for circuit in pol poq poc zksign; do
        if [ -d "$INSTALL_DIR/$circuit" ]; then
            echo "  • $circuit"
        fi
    done

    echo
    print_info "You can now run nomos with the NOMOS_CIRCUITS environment variable:"
    print_info "  export NOMOS_CIRCUITS=$INSTALL_DIR"
    print_info "Or nomos will automatically use ~/.nomos-circuits/"
    echo
}

# Run main
main
