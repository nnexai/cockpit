#!/usr/bin/env bash
set -euo pipefail

readonly CEF_BASE_URL='https://cef-builds.spotifycdn.com/'
readonly CEF_ARTIFACT='cef_binary_150.0.20+ga832838+chromium-150.0.7871.253_linux64_minimal.tar.bz2'
readonly CEF_SHA1='5dfbe1331a17a921c57a4ebbf334646d55c2e1b0'
readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly POC_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
readonly DESTINATION="${1:-${POC_ROOT}/.cef}"
readonly DOWNLOAD_DIR="${CEF_DOWNLOAD_DIR:-${POC_ROOT}/.downloads}"
readonly ARCHIVE_PATH="${DOWNLOAD_DIR}/${CEF_ARTIFACT}"

if ! command -v sha1sum >/dev/null 2>&1; then
  printf 'fetch-cef.sh: sha1sum is required to verify the pinned SDK\n' >&2
  exit 1
fi
if ! command -v tar >/dev/null 2>&1; then
  printf 'fetch-cef.sh: tar is required to unpack the pinned SDK\n' >&2
  exit 1
fi
if ! command -v curl >/dev/null 2>&1 && ! command -v wget >/dev/null 2>&1; then
  printf 'fetch-cef.sh: curl or wget is required to download the pinned SDK\n' >&2
  exit 1
fi

mkdir -p "${DOWNLOAD_DIR}"
if [[ ! -f "${ARCHIVE_PATH}" ]]; then
  printf 'Downloading %s\n' "${CEF_BASE_URL}${CEF_ARTIFACT}"
  if command -v curl >/dev/null 2>&1; then
    curl --fail --location --retry 3 --output "${ARCHIVE_PATH}.tmp" \
      "${CEF_BASE_URL}${CEF_ARTIFACT}"
  else
    wget --https-only --output-document="${ARCHIVE_PATH}.tmp" \
      "${CEF_BASE_URL}${CEF_ARTIFACT}"
  fi
  mv -- "${ARCHIVE_PATH}.tmp" "${ARCHIVE_PATH}"
fi

actual_sha1="$(sha1sum "${ARCHIVE_PATH}" | cut -d' ' -f1)"
mkdir -p "$(dirname -- "${DESTINATION}")"
if [[ "${actual_sha1}" != "${CEF_SHA1}" ]]; then
  printf 'fetch-cef.sh: SHA1 mismatch for %s\nexpected: %s\nactual:   %s\n' \
    "${ARCHIVE_PATH}" "${CEF_SHA1}" "${actual_sha1}" >&2
  exit 1
fi

if [[ -e "${DESTINATION}" ]]; then
  if [[ -f "${DESTINATION}/include/cef_app.h" ]]; then
    printf 'CEF SDK already unpacked and verified at %s\n' "${DESTINATION}"
    exit 0
  fi
  printf 'fetch-cef.sh: destination exists but is not a CEF SDK: %s\n' "${DESTINATION}" >&2
  exit 1
fi

mkdir -p "${DESTINATION}"
tmp_extract="$(mktemp -d "${DESTINATION}.extract.XXXXXX")"
cleanup() { rm -rf -- "${tmp_extract}"; }
trap cleanup EXIT

tar --extract --bzip2 --file "${ARCHIVE_PATH}" --directory "${tmp_extract}"
shopt -s nullglob
entries=("${tmp_extract}"/*)
if [[ "${#entries[@]}" -ne 1 || ! -d "${entries[0]}" ]]; then
  printf 'fetch-cef.sh: unexpected archive layout; expected one SDK directory\n' >&2
  exit 1
fi
mv -- "${entries[0]}"/* "${DESTINATION}/"
rmdir -- "${entries[0]}"
printf 'CEF SDK unpacked at %s (SHA1 %s)\n' "${DESTINATION}" "${actual_sha1}"
