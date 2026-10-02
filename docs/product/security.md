# Security and trust model

`ayni check`, `ayni verify`, and `ayni impact run` execute in the current
checkout and current execution environment. Ayni records whether that
environment is ordinary local execution or a verified Ayni image. It does not
claim that either one isolates repository code, controls a launcher, or makes
test results trustworthy.

CI systems and development platforms own sandboxing, network policy, resource
limits, credentials, checkout mounting, and container launch settings. Apply
those controls at the runner or platform boundary.

## Development images

`ayni env build` constructs a source-independent image from an explicit lock.
The image contains locked tools, dependency seeds, its runtime metadata, and
the Ayni executable. It never publishes or launches that image.

Every image contains a protected-content manifest and runtime metadata. With
no signing variables it is an unsigned image: verification proves local
consistency with the lock and protected content. With both signing variables,
verification also checks the repository-pinned public key and records the
trusted builder identity. A missing, invalid, or untrusted signature fails
explicitly.

Platforms attach the writable checkout before startup and invoke the image
entrypoint. The entrypoint prepares dependencies only from the image's locked
seeds and caches. It never replaces source files, refreshes native locks, or
silently removes dependency directories.

## Repository trust

An environment image executes package-manager preparation and repository tool
code while it is built. Review changes to the environment configuration, lock,
base image, dependencies, and signing setup as supply-chain changes. Use
ordinary registry and container controls for publishing, pulling by digest,
and platform-specific launch policy.
