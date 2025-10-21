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

# GPG Configuration
GPG_KEY_ID="50DC7A8DC24C5667"
GPG_PUBLIC_KEY="-----BEGIN PGP PUBLIC KEY BLOCK-----
 
mDMEZig60RYJKwYBBAHaRw8BAQdAy/+G05U5/EOA72WlcD4WkYn5SInri8pc4Z6D
BKNNGOm0JEFtYXpvbiBRIENMSSBUZWFtIDxxLWNsaUBhbWF6b24uY29tPoiZBBMW
CgBBFiEEmvYEF+gnQskUPgPsUNx6jcJMVmcFAmYoOtECGwMFCQPCZwAFCwkIBwIC
IgIGFQoJCAsCBBYCAwECHgcCF4AACgkQUNx6jcJMVmef5QD/QWWEGG/cOnbDnp68
SJXuFkwiNwlH2rPw9ZRIQMnfAS0A/0V6ZsGB4kOylBfc7CNfzRFGtovdBBgHqA6P
zQ/PNscGuDgEZig60RIKKwYBBAGXVQEFAQEHQC4qleONMBCq3+wJwbZSr0vbuRba
D1xr4wUPn4Avn4AnAwEIB4h+BBgWCgAmFiEEmvYEF+gnQskUPgPsUNx6jcJMVmcF
AmYoOtECGwwFCQPCZwAACgkQUNx6jcJMVmchMgEA6l3RveCM0YHAGQaSFMkguoAo
vK6FgOkDawgP0NPIP2oA/jIAO4gsAntuQgMOsPunEdDeji2t+AhV02+DQIsXZpoB
=f8yY
-----END PGP PUBLIC KEY BLOCK-----"

# Global variables
use_musl=false
skip_gpg=false
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
            --skip-gpg)
                skip_gpg=true
                shift
                ;;
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
    --skip-gpg    Skip GPG signature verification (not recommended)
    --help, -h    Show this help message

This script will:
1. Detect your platform and architecture
2. Download the appropriate $CLI_NAME package
3. Verify checksums and GPG signatures (if available)
4. Install $CLI_NAME on your system
5. Set up shell integrations

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
    
    # Check for GPG (optional)
    if [[ "$skip_gpg" == "false" ]] && ! command -v gpg >/dev/null 2>&1; then
        warning "GPG not found. Signature verification will be skipped."
        warning "Install gpg for enhanced security verification."
        skip_gpg=true
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
# GPG Functions
# =============================================================================

setup_gpg() {
    if [[ "$skip_gpg" == "true" ]]; then
        return 0
    fi
    
    log "Setting up GPG verification..."
    
    # Create download directory if it doesn't exist
    mkdir -p "$DOWNLOAD_DIR"
    
    # Create temporary key file
    local key_file="$DOWNLOAD_DIR/amazon-q-public.key"
    echo "$GPG_PUBLIC_KEY" > "$key_file"
    downloaded_files+=("$key_file")
    
    # Import the key
    if gpg --import "$key_file" >/dev/null 2>&1; then
        success "Amazon Q public key imported"
    else
        warning "Failed to import GPG key. Skipping signature verification."
        skip_gpg=true
    fi
}

verify_gpg_signature() {
    if [[ "$skip_gpg" == "true" ]]; then
        return 0
    fi
    
    local file_path="$1"
    local sig_url="$2"
    local sig_path="${file_path}.sig"
    
    log "Downloading GPG signature..."
    if ! download_file "$sig_url" "$sig_path"; then
        warning "Failed to download signature file. Skipping GPG verification."
        return 0
    fi
    
    downloaded_files+=("$sig_path")
    
    log "Verifying GPG signature..."
    if gpg --verify "$sig_path" "$file_path" >/dev/null 2>&1; then
        success "GPG signature verified successfully"
        return 0
    else
        warning "GPG signature verification failed"
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
        sig_url="" # No GPG signature for DMG files
    else
        # Linux
        if [[ "$use_musl" == "true" ]]; then
            filename="q-${arch}-linux-musl.zip"
        else
            filename="q-${arch}-linux.zip"
        fi
        download_url="${BASE_URL}/latest/$filename"
        sig_url="${download_url}.sig"
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
    
    # Verify GPG signature for Linux packages
    if [[ "$os" == "linux" ]] && [[ -n "$sig_url" ]]; then
        verify_gpg_signature "$file_path" "$sig_url"
    fi
    
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
    
    # Set up GPG if available
    if [[ "$os" == "linux" ]]; then
        setup_gpg
    fi
    
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
    
    if [[ "$skip_gpg" == "true" ]]; then
        warning "GPG signature verification was skipped."
        echo "   For enhanced security, install gpg and run this script again."
        echo
    fi
}

# Run main function
main "$@"
