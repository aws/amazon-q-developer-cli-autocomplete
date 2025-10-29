#!/bin/bash

set -eu

# =============================================================================
# Q CLI Installation Script
# =============================================================================

# Configuration
BINARY_NAME="q"
CLI_NAME="Q CLI"
COMMAND_NAME="q"
DESKTOP_BINARY_NAME="q_desktop"
BASE_URL="https://desktop-release.q.us-east-1.amazonaws.com"
MANIFEST_URL="${BASE_URL}/latest/manifest.json"
MACOS_FILENAME="Amazon Q.dmg"
MACOS_FILENAME_ESCAPED="Amazon%20Q.dmg"

# Installation directories
MACOS_APP_DIR="/Applications"
LINUX_INSTALL_DIR="$HOME/.local/bin"
DOWNLOAD_DIR="$HOME/.${BINARY_NAME}/downloads"

# Global variables
use_musl=false
downloaded_files=()
temp_dirs=()
mounted_dmg=""
SUCCESS=false

# =============================================================================
# Utility Functions
# =============================================================================

log() {
    echo "🔧 $1" >&2
}

success() {
    echo "🎉 $1" >&2
}

error() {
    echo "❌ Error: $1" >&2
    exit 1
}

warning() {
    echo "⚠️  Warning: $1" >&2
}

# Parse command line arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --help|-h)
                show_help
                exit 0
                ;;
            *)
                error "Unknown option: $1"
                ;;
        esac
    done
}

show_help() {
    cat << EOF
$CLI_NAME Installation Script

Usage: $0 [OPTIONS]

Options:
    --help, -h    Show this help message

This script will:
1. Detect your platform and architecture
2. Download the appropriate $CLI_NAME package
3. Verify checksums
4. Install $CLI_NAME on your system

For more information, visit: https://docs.aws.amazon.com/amazonq/latest/qdeveloper-ug/command-line-installing.html
EOF
}

# Check for required dependencies
check_dependencies() {
    local missing_deps=()
    
    # Check for downloader
    if ! command -v curl >/dev/null 2>&1 && ! command -v wget >/dev/null 2>&1; then
        missing_deps+=("curl or wget")
    fi
    
    # Check for unzip on Linux
    if [[ "$os" == "linux" ]] && ! command -v unzip >/dev/null 2>&1; then
        missing_deps+=("unzip")
    fi
    
    # Check for shasum/sha256sum
    if [[ "$os" == "darwin" ]] && ! command -v shasum >/dev/null 2>&1; then
        missing_deps+=("shasum")
    elif [[ "$os" == "linux" ]] && ! command -v sha256sum >/dev/null 2>&1; then
        missing_deps+=("sha256sum")
    fi
    
    if [[ ${#missing_deps[@]} -gt 0 ]]; then
        error "Missing required dependencies: ${missing_deps[*]}"
    fi
}

# Download function that works with both curl and wget
download_file() {
    local url="$1"
    local output="${2:-}"
    
    if command -v curl >/dev/null 2>&1; then
        if [[ -n "$output" ]]; then
            curl -fsSL -o "$output" "$url" || error "Failed to download $url"
        else
            curl -fsSL "$url" || error "Failed to download $url"
        fi
    elif command -v wget >/dev/null 2>&1; then
        if [[ -n "$output" ]]; then
            wget -q -O "$output" "$url" || error "Failed to download $url"
        else
            wget -q -O - "$url" || error "Failed to download $url"
        fi
    else
        error "No downloader available"
    fi
}

# Get checksum from manifest.json
get_checksum() {
    local json="$1"
    local filename="$2"
    
    if command -v jq >/dev/null 2>&1; then
        # Use jq to find the package with matching download filename
        echo "$json" | jq -r ".packages[] | select(.download | endswith(\"$filename\")) | .sha256 // empty"
    else
        # Fallback: parse JSON manually
        # Normalize to single line
        local package_obj
        package_obj=$(echo "$json" | tr -d '\n\r' | grep -o '{[^}]*"download"[^}]*'"$filename"'[^}]*}')

        if [[ -n "$package_obj" ]]; then
            if [[ $package_obj =~ \"sha256\"[[:space:]]*:[[:space:]]*\"([a-f0-9]{64})\" ]]; then
                echo "${BASH_REMATCH[1]}"
                return 0
            fi
        fi

        return 1
    fi
}

# =============================================================================
# Platform Detection
# =============================================================================

detect_platform() {
    case "$(uname -s)" in
        Darwin) os="darwin" ;;
        Linux) os="linux" ;;
        *) error "Unsupported operating system: $(uname -s)" ;;
    esac

    case "$(uname -m)" in
        x86_64|amd64) arch="x86_64" ;;
        arm64|aarch64) arch="aarch64" ;;
        *) error "Unsupported architecture: $(uname -m)" ;;
    esac
}

# Minimum required glibc version
GLIBC_MIN_MAJOR=2
GLIBC_MIN_MINOR=34

# Check if a glibc version meets the minimum requirement
is_glibc_version_sufficient() {
    local version="$1"
    local major minor

    IFS='.' read -r major minor <<EOF
$version
EOF
    if [[ -z "$minor" ]]; then
        minor=0
    fi

    if (( major > GLIBC_MIN_MAJOR || (major == GLIBC_MIN_MAJOR && minor >= GLIBC_MIN_MINOR) )); then
        return 0
    else
        return 1
    fi
}

# Check glibc version for Linux
check_glibc() {
    if [[ "$os" != "linux" ]]; then
        return 0
    fi
    
    local glibc_version

    # Method 1: Try common libc.so.6 locations
    for LIBC_PATH in /lib64/libc.so.6 /lib/libc.so.6 /usr/lib/x86_64-linux-gnu/libc.so.6 \
        /lib/aarch64-linux-gnu/libc.so.6; do
        if [[ -f "$LIBC_PATH" ]]; then
            glibc_version=$("$LIBC_PATH" | sed -n 's/^GNU C Library (.*) stable release version \([0-9]*\)\.\([0-9]*\).*$/\1.\2/p')
            if [[ -n "$glibc_version" ]]; then
                if is_glibc_version_sufficient "$glibc_version"; then
                    return 0
                else
                    use_musl=true
                    return 0
                fi
            fi
        fi
    done

    # Method 2: Try ldd --version as a more reliable alternative
    if command -v ldd >/dev/null 2>&1; then
        glibc_version=$(ldd --version 2>/dev/null | head -n 1 | grep -o '[0-9]\+\.[0-9]\+' | head -n 1)
        if [[ -n "$glibc_version" ]]; then
            if is_glibc_version_sufficient "$glibc_version"; then
                return 0
            else
                use_musl=true
                return 0
            fi
        fi
    fi

    # Method 3: Try getconf as a fallback
    if command -v getconf >/dev/null 2>&1; then
        glibc_version=$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}')
        if [[ -n "$glibc_version" ]]; then
            if is_glibc_version_sufficient "$glibc_version"; then
                return 0
            else
                use_musl=true
                return 0
            fi
        fi
    fi

    # Check for musl directly
    if [[ -f /lib/libc.musl-x86_64.so.1 ]] || [[ -f /lib/libc.musl-aarch64.so.1 ]] || \
       ldd /bin/ls 2>&1 | grep -q musl; then
        use_musl=true
        return 0
    fi

    use_musl=true
    return 0
}

# =============================================================================
# Download and Installation Functions
# =============================================================================

# Download and verify file
download_and_verify() {
    local download_url="$1"
    local filename="$2"

    mkdir -p "$DOWNLOAD_DIR"

    local file_path="$DOWNLOAD_DIR/$filename"
    downloaded_files+=("$file_path")

    log "Downloading $CLI_NAME..."
    download_file "$download_url" "$file_path"

    log "Verifying download..."
    local manifest_json
    manifest_json=$(download_file "$MANIFEST_URL")

    local expected_checksum
    expected_checksum=$(get_checksum "$manifest_json" "$filename")

    if [[ -z "$expected_checksum" ]] || [[ ! "$expected_checksum" =~ ^[a-f0-9]{64}$ ]]; then
        error "Could not find valid checksum for $filename"
    fi

    local actual_checksum
    if [[ "$os" == "darwin" ]]; then
        actual_checksum=$(shasum -a 256 "$file_path" | cut -d' ' -f1)
    else
        actual_checksum=$(sha256sum "$file_path" | cut -d' ' -f1)
    fi

    if [[ "$actual_checksum" != "$expected_checksum" ]]; then
        rm -f "$file_path"
        error "Checksum verification failed. Expected: $expected_checksum, Got: $actual_checksum"
    fi
}

# Create symlink, overwriting if it exists and is invalid
create_symlink() {
    local src="$1"
    local dst="$2"

    # Check if link already exists and points to the right place
    if [[ -L "$dst" ]]; then
        local current_target
        current_target=$(readlink "$dst")
        if [[ "$current_target" == "$src" ]]; then
            return 0
        fi
        rm -f "$dst"
    elif [[ -e "$dst" ]]; then
        rm -f "$dst"
    fi

    ln -s "$src" "$dst"
}

# Install on macOS
install_macos() {
    local dmg_path="$1"
    if [[ ! -f "$dmg_path" ]]; then
        error "DMG file not found: $dmg_path"
    fi

    local mount_path
    mount_path=$(hdiutil attach "$dmg_path" -nobrowse -readonly | grep Volumes | cut -f 3)
    if [[ -z "$mount_path" ]]; then
        error "Failed to mount DMG"
    fi
    mounted_dmg="$mount_path"
    
    # Find the .app bundle
    local app_bundle
    app_bundle=$(find "$mount_path" -name "*.app" -maxdepth 1 -type d | head -1)
    
    if [[ -z "$app_bundle" ]]; then
        error "Could not find application bundle in DMG"
    fi
    
    local app_name
    app_name=$(basename "$app_bundle")
    
    # Check if app already exists and warn user
    if [[ -d "$MACOS_APP_DIR/$app_name" ]]; then
        warning "Existing $app_name found in $MACOS_APP_DIR"
        echo "Do you want to replace it? (y/N): "
        read -r response
        if [[ ! "$response" =~ ^[Yy]$ ]]; then
            error "Installation cancelled by user"
        fi
        log "Replacing existing $app_name..."
        rm -rf "$MACOS_APP_DIR/$app_name"
    fi
    
    cp -R "$app_bundle" "$MACOS_APP_DIR/"
    
    mkdir -p "$HOME/.local/bin"
    local macos_bin="$MACOS_APP_DIR/$app_name/Contents/MacOS"

    "$macos_bin/$DESKTOP_BINARY_NAME" --no-dashboard > /dev/null 2>&1 &
}

# Install on Linux
install_linux() {
    local zip_path="$1"
    
    log "Extracting archive..."
    local extract_dir="$DOWNLOAD_DIR/extract"
    mkdir -p "$extract_dir"
    temp_dirs+=("$extract_dir")
    
    unzip -q "$zip_path" -d "$extract_dir"
    
    # Find and run the install script
    local install_script="$extract_dir/${BINARY_NAME}/install.sh"
    
    if [[ ! -f "$install_script" ]]; then
        error "Install script not found in archive"
    fi
    
    log "Running installer..."
    chmod +x "$install_script"
    Q_SKIP_SETUP=1 "$install_script"
}

# Cleanup function - only removes files/dirs we created
cleanup() {
    if [ "$SUCCESS" = false ]; then
        error "Installation failed. Cleaning up..."
    fi

    # Detach mounted DMG if any
    if [[ -n "$mounted_dmg" ]]; then
        hdiutil detach "$mounted_dmg" -quiet 2>/dev/null || true
    fi

    # Remove downloaded files
    if [[ ${#downloaded_files[@]} -gt 0 ]]; then
        for file in "${downloaded_files[@]}"; do
            if [[ -f "$file" ]]; then
                rm -f "$file"
            fi
        done
    fi

    # Remove temporary directories we created
    if [[ ${#temp_dirs[@]} -gt 0 ]]; then
        for dir in "${temp_dirs[@]}"; do
            if [[ -d "$dir" ]]; then
                rm -rf "$dir"
            fi
        done
    fi
}

# =============================================================================
# Main Installation Process
# =============================================================================

main() {
    log "Installing $CLI_NAME..."
    
    # Parse command line arguments
    parse_args "$@"
    
    # Set up cleanup trap
    trap cleanup EXIT
    
    # Platform detection and validation
    detect_platform
    check_dependencies
    check_glibc
    
    # Get download information
    local download_url filename
    if [[ "$os" == "darwin" ]]; then
        filename="$MACOS_FILENAME"
        download_url="${BASE_URL}/latest/${MACOS_FILENAME_ESCAPED}"
    else
        # Linux
        if [[ "$use_musl" == "true" ]]; then
            filename="q-${arch}-linux-musl.zip"
        else
            filename="q-${arch}-linux.zip"
        fi
        download_url="${BASE_URL}/latest/$filename"
    fi
    
    # Download and verify
    download_and_verify "$download_url" "$filename"
    local downloaded_file="$DOWNLOAD_DIR/$filename"

    # Install based on platform
    if [[ "$os" == "darwin" ]]; then
        install_macos "$downloaded_file"
    else
        install_linux "$downloaded_file"
    fi
    
    SUCCESS=true
    
    echo
    success "$CLI_NAME installation completed successfully!"
    echo

    # Check if ~/.local/bin is on PATH
    if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
        warning "$HOME/.local/bin is not on your PATH"
        echo "Add it to your PATH by adding this line to your shell configuration file:"
        echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
        echo
    fi

    echo "Next steps:"
    echo "1. Run: $COMMAND_NAME --help to get started"
    echo "2. Run: $COMMAND_NAME chat to start an interactive session"
    echo
}

# Run main function
main "$@"
