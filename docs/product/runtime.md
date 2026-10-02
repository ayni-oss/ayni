# Quality runtime

Quality commands run directly in the current checkout. They do not create a
container, copy the repository, re-execute Ayni, or install dependencies.

On an ordinary machine, Ayni validates the configured targets and invokes the
available native tools. When `/etc/ayni/runtime.json` is present, Ayni verifies
that runtime metadata, the checkout lock, platform, protected content, and
optional signature before it activates the corresponding locked tool versions.
Malformed, stale, incompatible, or tampered metadata fails explicitly.

An Ayni image has one entrypoint for platform launch. The platform attaches the
writable checkout first. Startup can materialize locked dependency seeds and
caches into that checkout without replacing source, refreshing native locks, or
silently deleting existing dependencies. Native development commands continue
to work from their project roots.

`ayni env doctor` diagnoses the lock and current image state. It does not
prepare dependencies or run quality work.
