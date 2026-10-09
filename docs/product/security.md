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

## Verification and evidence

A valid signature authenticates a statement from a key; consumers must also trust that key and match the expected artifact and inputs.
For repository environments, verify the lock, platform, certificate, and protected content under the configured trust policy.
The launching platform owns admission of the expected immutable image digest.

Builder release provenance and repository environment certification describe different artifacts.
The versioned factory builds the repository image; verifying the latter does not establish the factory's release provenance.
[Feature #75](https://github.com/ayni-oss/ayni/issues/75) tracks verification before builder work starts, including the trusted launcher's role.
The current factory does not implement that startup gate; self-verification inside an untrusted image is not a sufficient root of trust.

Quality results measure the configured tools and tests against repository policy.
They do not prove universal correctness, independent approval, or regulatory compliance.
A platform requiring independent assessment must control its policy, execution identity, and retained evidence outside the code author's authority.
