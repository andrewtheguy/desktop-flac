#!/usr/bin/env bash
# Run the tests and clippy against each libFLAC the crate loads, in a Debian
# container that has it: FLAC 1.5 is trixie's libflac14, FLAC 1.4 bookworm's
# libflac12. With no argument, both. Runs on the host's architecture only; each
# release builds into its own folder under target/.
#
#   ./scripts/test-libflac.sh [1.5|1.4]...
set -euo pipefail

cd "$(dirname "$0")/.."
[[ $# -gt 0 ]] || set -- 1.5 1.4

for flac in "$@"; do
	case "${flac}" in
		1.5) suite=trixie package=libflac14 ;;
		1.4) suite=bookworm package=libflac12 ;;
		*) echo "usage: $0 [1.5|1.4]..." >&2; exit 2 ;;
	esac
	target="target/libflac-${flac}"
	mkdir -p "${target}"
	echo "== FLAC ${flac}: ${package} on ${suite}"
	podman run --rm --pull=newer -v "${PWD}:/src:ro" -v "${PWD}/${target}:/target" -w /src \
		-e CARGO_TARGET_DIR=/target -e DEBIAN_FRONTEND=noninteractive -e FLAC="${flac}" -e PACKAGE="${package}" \
		"docker.io/library/rust:${suite}" bash -euo pipefail -c '
			apt-get update -qq
			apt-get install -y -qq --no-install-recommends "${PACKAGE}" >/dev/null
			ver="$(dpkg-query -W -f="\${Version}" "${PACKAGE}")"
			[[ "${ver}" == "${FLAC}".* ]] || { echo "${PACKAGE} is ${ver}, not FLAC ${FLAC}" >&2; exit 1; }
			echo "${PACKAGE} ${ver}"
			rustup component add clippy >/dev/null 2>&1
			cargo test
			cargo clippy --all-targets -- -D warnings
		'
done
