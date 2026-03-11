#!/bin/sh
# Metis installer script
# Usage: curl -fsSL https://get.metis.dev | bash
#   or:  curl -fsSL https://get.metis.dev | bash -s -- --version v0.1.0
#
# Downloads the correct pre-built metis binary for the current OS and
# architecture, verifies its SHA256 checksum, and installs it to ~/.metis/bin/.

set -eu

GITHUB_REPO="dourolabs/metis-releases"
INSTALL_DIR="${HOME}/.metis/bin"
BINARY_NAME="metis"

main() {
    parse_args "$@"
    detect_platform
    setup_downloader

    local version
    version="${VERSION:-latest}"

    printf "Installing metis (%s) for %s...\n" "${version}" "${TARGET}"

    local tmp_dir
    tmp_dir="$(mktemp -d)"
    trap 'rm -rf "${tmp_dir}"' EXIT

    local artifact_name="metis-single-player-${TARGET}"
    local binary_url checksums_url

    if [ "${version}" = "latest" ]; then
        binary_url="https://github.com/${GITHUB_REPO}/releases/latest/download/${artifact_name}"
        checksums_url="https://github.com/${GITHUB_REPO}/releases/latest/download/SHA256SUMS.txt"
    else
        binary_url="https://github.com/${GITHUB_REPO}/releases/download/${version}/${artifact_name}"
        checksums_url="https://github.com/${GITHUB_REPO}/releases/download/${version}/SHA256SUMS.txt"
    fi

    printf "Downloading binary...\n"
    download "${binary_url}" "${tmp_dir}/${artifact_name}"

    printf "Downloading checksums...\n"
    download "${checksums_url}" "${tmp_dir}/SHA256SUMS.txt"

    printf "Verifying checksum...\n"
    verify_checksum "${tmp_dir}" "${artifact_name}"

    printf "Installing to %s...\n" "${INSTALL_DIR}"
    mkdir -p "${INSTALL_DIR}"
    cp "${tmp_dir}/${artifact_name}" "${INSTALL_DIR}/${BINARY_NAME}"
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

    add_to_path

    printf "\nmetis has been installed to %s/%s\n" "${INSTALL_DIR}" "${BINARY_NAME}"
    printf "\nTo get started, run:\n"
    printf "  metis server init\n"
    printf "\nIf '%s' is not in your PATH, restart your shell or run:\n" "${INSTALL_DIR}"
    printf "  export PATH=\"%s:\$PATH\"\n" "${INSTALL_DIR}"
}

parse_args() {
    VERSION=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --version)
                shift
                if [ $# -eq 0 ]; then
                    error "--version requires a value (e.g., --version v0.1.0)"
                fi
                VERSION="$1"
                ;;
            --help)
                printf "Usage: install.sh [--version VERSION]\n"
                printf "\nOptions:\n"
                printf "  --version VERSION  Install a specific version (e.g., v0.1.0). Defaults to latest.\n"
                printf "  --help             Show this help message.\n"
                exit 0
                ;;
            *)
                error "Unknown option: $1. Use --help for usage information."
                ;;
        esac
        shift
    done
}

detect_platform() {
    local os arch

    os="$(uname -s)"
    arch="$(uname -m)"

    case "${os}" in
        Linux)  os="unknown-linux-gnu" ;;
        Darwin) os="apple-darwin" ;;
        *)      error "Unsupported operating system: ${os}. Only Linux and macOS are supported." ;;
    esac

    case "${arch}" in
        x86_64|amd64)   arch="x86_64" ;;
        aarch64|arm64)  arch="aarch64" ;;
        *)              error "Unsupported architecture: ${arch}. Only x86_64 and aarch64 are supported." ;;
    esac

    TARGET="${arch}-${os}"
}

setup_downloader() {
    if command -v curl >/dev/null 2>&1; then
        DOWNLOADER="curl"
    elif command -v wget >/dev/null 2>&1; then
        DOWNLOADER="wget"
    else
        error "Either curl or wget is required to download files."
    fi
}

download() {
    local url="$1"
    local output="$2"

    case "${DOWNLOADER}" in
        curl)
            if ! curl -fsSL -o "${output}" "${url}"; then
                error "Failed to download: ${url}"
            fi
            ;;
        wget)
            if ! wget -q -O "${output}" "${url}"; then
                error "Failed to download: ${url}"
            fi
            ;;
    esac
}

verify_checksum() {
    local dir="$1"
    local artifact="$2"

    local expected
    expected="$(grep "${artifact}" "${dir}/SHA256SUMS.txt" | awk '{print $1}')"

    if [ -z "${expected}" ]; then
        error "Checksum not found for ${artifact} in SHA256SUMS.txt"
    fi

    local actual
    if command -v sha256sum >/dev/null 2>&1; then
        actual="$(sha256sum "${dir}/${artifact}" | awk '{print $1}')"
    elif command -v shasum >/dev/null 2>&1; then
        actual="$(shasum -a 256 "${dir}/${artifact}" | awk '{print $1}')"
    else
        printf "Warning: sha256sum/shasum not found, skipping checksum verification.\n"
        return 0
    fi

    if [ "${actual}" != "${expected}" ]; then
        error "Checksum verification failed.\n  Expected: ${expected}\n  Actual:   ${actual}"
    fi

    printf "Checksum verified.\n"
}

add_to_path() {
    # If already in PATH, nothing to do
    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*) return ;;
    esac

    local shell_profile=""
    local current_shell
    current_shell="$(basename "${SHELL:-/bin/sh}")"

    case "${current_shell}" in
        zsh)  shell_profile="${HOME}/.zshrc" ;;
        bash)
            if [ -f "${HOME}/.bash_profile" ]; then
                shell_profile="${HOME}/.bash_profile"
            else
                shell_profile="${HOME}/.bashrc"
            fi
            ;;
        fish) shell_profile="${HOME}/.config/fish/config.fish" ;;
        *)    shell_profile="${HOME}/.profile" ;;
    esac

    if [ -n "${shell_profile}" ]; then
        local path_line="export PATH=\"${INSTALL_DIR}:\$PATH\""
        if [ "${current_shell}" = "fish" ]; then
            path_line="set -gx PATH \"${INSTALL_DIR}\" \$PATH"
        fi

        # Check if already added (idempotent)
        if [ -f "${shell_profile}" ] && grep -qF "${INSTALL_DIR}" "${shell_profile}" 2>/dev/null; then
            return
        fi

        printf "\n# Added by metis installer\n%s\n" "${path_line}" >> "${shell_profile}"
        printf "Added %s to PATH in %s\n" "${INSTALL_DIR}" "${shell_profile}"
    fi
}

error() {
    printf "Error: %s\n" "$1" >&2
    exit 1
}

main "$@"
