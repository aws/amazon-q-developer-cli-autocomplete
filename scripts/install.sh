#!/bin/bash

set -e

# =============================================================================
# Q CLI Installation Script
# =============================================================================

# Configuration
BINARY_NAME="q"
CLI_NAME="Q CLI"
COMMAND_NAME="q"
BASE_URL="https://desktop-release.q.us-east-1.amazonaws.com"
INDEX_URL="${BASE_URL}/index.json"

# Installation directories
MACOS_APP_DIR="/Applications"
LINUX_INSTALL_DIR="$HOME/.local/bin"
DOWNLOAD_DIR="$HOME/.${BINARY_NAME}/downloads"

# Global variables
use_musl=false
downloaded_files=()
temp_dirs=()

# =============================================================================
# Utility Functions
# =============================================================================

log() {
    echo "🔧 $1" >&2
}

success() {
    echo "✅ $1" >&2
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
    local output="$2"
    
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

# Simple JSON parser for when jq is not available
parse_json_value() {
    local json="$1"
    local key="$2"
    
    # Remove whitespace and extract value
    echo "$json" | sed -n "s/.*\"$key\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p" | head -1
}

# Get checksum from index.json
get_checksum() {
    local json="$1"
    local filename="$2"
    
    if command -v jq >/dev/null 2>&1; then
        # Use jq to find the package with matching download filename from the latest version
        echo "$json" | jq -r ".versions[-1].packages[] | select(.download | endswith(\"$filename\")) | .sha256 // empty"
    else
        error "jq is required for checksum verification. Please install jq and try again."
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

    log "Detected platform: $os-$arch"
}

# Check glibc version for Linux
check_glibc() {
    if [[ "$os" != "linux" ]]; then
        return 0
    fi
    
    local glibc_version
    if command -v ldd >/dev/null 2>&1; then
        glibc_version=$(ldd --version 2>/dev/null | head -1 | grep -o '[0-9]\+\.[0-9]\+' | head -1)
        log "Detected glibc version: $glibc_version"
        
        # Check if we need musl version (glibc < 2.34)
        if [[ -n "$glibc_version" ]]; then
            local major minor
            IFS='.' read -r major minor <<< "$glibc_version"
            if (( major < 2 || (major == 2 && minor < 34) )); then
                use_musl=true
                log "Using musl version for older glibc"
            fi
        fi
    else
        # Check for musl directly
        if [[ -f /lib/libc.musl-x86_64.so.1 ]] || [[ -f /lib/libc.musl-aarch64.so.1 ]] || \
           ldd /bin/ls 2>&1 | grep -q musl; then
            use_musl=true
            log "Detected musl system"
        fi
    fi
}

# =============================================================================
# Download and Installation Functions
# =============================================================================

# Get download URL and filename based on platform
get_download_info() {
    if [[ "$os" == "darwin" ]]; then
        filename="Amazon Q.dmg"
        download_url="${BASE_URL}/latest/Amazon%20Q.dmg"
    else
        # Linux
        if [[ "$use_musl" == "true" ]]; then
            filename="q-${arch}-linux-musl.zip"
        else
            filename="q-${arch}-linux.zip"
        fi
        download_url="${BASE_URL}/latest/$filename"
    fi
    
    log "Download URL: $download_url"
    log "Filename: $filename"
}

# Download and verify file
download_and_verify() {
    mkdir -p "$DOWNLOAD_DIR"
    
    local file_path="$DOWNLOAD_DIR/$filename"
    downloaded_files+=("$file_path")
    
    log "Downloading $CLI_NAME to $file_path..."
    download_file "$download_url" "$file_path"
    
    log "Downloading index for verification..."
    local index_json
    index_json=$(download_file "$INDEX_URL")
    
    log "Verifying checksum..."
    local expected_checksum
    expected_checksum=$(get_checksum "$index_json" "$filename")
    
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
    
    success "Checksum verified successfully"
    
    echo "$file_path"
}

# Install on macOS
install_macos() {
    log "Mounting DMG..."
    local dmg_path="$1"
    if [[ ! -f "$dmg_path" ]]; then
        error "DMG file not found: $dmg_path"
    fi

    local mount_path
    mount_path=$(hdiutil attach "$dmg_path" -nobrowse | grep Volumes | cut -f 3)
    if [[ -z "$mount_path" ]]; then
        error "Failed to mount DMG"
    fi
    
    # Find the .app bundle
    local app_bundle
    echo "Mount Point: $mount_path"
    app_bundle=$(find "$mount_path" -name "*.app" -maxdepth 1 -type d | head -1)
    
    if [[ -z "$app_bundle" ]]; then
        hdiutil detach "$mount_path" -quiet
        error "Could not find application bundle in DMG"
    fi
    
    local app_name
    app_name=$(basename "$app_bundle")
    
    log "Installing $app_name to $MACOS_APP_DIR..."
    
    # Check if app already exists and warn user
    if [[ -d "$MACOS_APP_DIR/$app_name" ]]; then
        warning "Existing $app_name found in $MACOS_APP_DIR"
        echo "Do you want to replace it? (y/N): "
        read -r response
        if [[ ! "$response" =~ ^[Yy]$ ]]; then
            hdiutil detach "$mount_path" -quiet
            error "Installation cancelled by user"
        fi
        log "Removing existing $app_name..."
        rm -rf "$MACOS_APP_DIR/$app_name"
    fi
    
    cp -R "$app_bundle" "$MACOS_APP_DIR/"

    success "$CLI_NAME installed successfully to $MACOS_APP_DIR"
    
    log "Unmounting DMG..."
    hdiutil detach "$mount_path" -quiet
    
    log "Starting $app_name..."
    open "$MACOS_APP_DIR/$app_name"
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
    "$install_script"
    
    success "$CLI_NAME installed successfully"
}

# # Install shell integrations
# install_integrations() {
#     log "Installing shell integrations..."
    
#     # Check if q command is available
#     if command -v "$COMMAND_NAME" >/dev/null 2>&1; then
#         "$COMMAND_NAME" integrations install all || warning "Failed to install shell integrations"
#         success "Shell integrations installed"
#     else
#         warning "Could not find $COMMAND_NAME command. You may need to restart your shell or add it to your PATH."
#         echo "After restarting your shell, run: $COMMAND_NAME integrations install"
#     fi
# }

# Cleanup function - only removes files/dirs we created
cleanup() {
    log "Cleaning up temporary files..."
    
    # Remove downloaded files
    for file in "${downloaded_files[@]}"; do
        if [[ -f "$file" ]]; then
            rm -f "$file"
        fi
    done
    
    # Remove temporary directories we created
    for dir in "${temp_dirs[@]}"; do
        if [[ -d "$dir" ]]; then
            rm -rf "$dir"
        fi
    done
}

# =============================================================================
# Main Installation Process
# =============================================================================

main() {
    echo "🚀 Installing $CLI_NAME..."
    echo
    
    # Parse command line arguments
    parse_args "$@"
    
    # Set up cleanup trap
    trap cleanup EXIT
    
    # Platform detection and validation
    detect_platform
    check_dependencies
    check_glibc
    
    # Get download information
    get_download_info
    
    # Download and verify
    download_and_verify
    downloaded_file="$DOWNLOAD_DIR/$filename"

    # Install based on platform
    if [[ "$os" == "darwin" ]]; then
        install_macos "$downloaded_file"
    else
        install_linux "$downloaded_file"
    fi
    
    
    echo
    success "🎉 $CLI_NAME installation completed successfully!"
    echo
    echo "Next steps:"
    echo "1. Run: $COMMAND_NAME --help to get started"
    echo "2. Run: $COMMAND_NAME chat to start an interactive session"
    echo "3. Run: $COMMAND_NAME integrations install --help to install terminal integrations"
    echo
}

# Run main function
main "$@"
