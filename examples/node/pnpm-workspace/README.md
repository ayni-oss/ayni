# Prepared workspace links

This fixture has a package-local external tool (`vitest`) and a `workspace:*`
dependency on editable project source. It exercises both link destinations in
one managed run.

Use a checkout-built Ayni CLI and matching Linux executor; no release is needed:

```sh
"$AYNI" env lock --repo-root examples/node/pnpm-workspace
"$AYNI" env build --repo-root examples/node/pnpm-workspace \
  --executor-image "$EXECUTOR" --tag local/prepared-links
```

Copy the fixture to a clean temporary directory (without `node_modules` or
`.ayni`), then run it offline at a path different from its build-time location:

```sh
docker run --rm --network none --user "$(id -u):$(id -g)" \
  --env HOME=/tmp/home --env XDG_CACHE_HOME=/tmp/cache \
  --env MISE_CACHE_DIR=/tmp/mise \
  --mount "type=bind,source=$FIXTURE,target=/checkout" --workdir /checkout \
  local/prepared-links ayni check
```

Repeat the command against the same checkout to verify reuse. Both runs should
report one passing test. An existing dependency directory without Ayni's
preparation marker must be rejected, not deleted. Keep the source directory
writable: changing the library implementation should change the test result
without rebuilding or recertifying the image.
