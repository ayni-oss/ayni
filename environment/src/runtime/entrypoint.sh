#!/bin/sh
set -eu

# Compute the locked tool paths from the protected image configuration. Do not
# use Mise shims here: a caller may use a different writable HOME or cache
# directory when it mounts a checkout into the image.
requested_cargo_home=${CARGO_HOME-}
export CARGO_HOME="${AYNI_RUNTIME_CARGO_HOME:?missing image Cargo home}"
environment="$(/usr/local/bin/mise -C /etc/ayni env -s bash)"
eval "$environment"

# The tool paths above retain the image's Cargo binaries. Restore an explicit
# caller cache afterwards so project dependency downloads and registries stay
# writable without changing how those binaries are resolved.
if [ -n "$requested_cargo_home" ]; then
    export CARGO_HOME="$requested_cargo_home"
fi

if [ "$#" -eq 0 ]; then
    set -- /bin/sh
fi
exec /usr/local/bin/ayni __exec "$@"
