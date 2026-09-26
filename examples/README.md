# Example fixtures for adapter and signal testing

Each language has:

- `single/`: one service repository **without** Ayni files preinstalled. Contains intentional signal failures.
- `mono/`: monorepo with `math` lib, `greeting` lib, and `greeting-service` app. Contains dependency/policy scenarios.

Layout:

- `examples/<language>/single`
- `examples/<language>/mono`

These fixtures are source fixtures, not per-language Docker harnesses. Preserve
both layouts when changing examples; run them through Ayni's managed environment
lifecycle or documented host prerequisites.

The `math` library exports 10 functions and includes tests for 8/10 to make coverage intentionally incomplete.

Kotlin examples use Gradle Kotlin DSL. `single/` is intentionally missing Ayni
files; `mono/` includes `.ayni.toml` and Gradle quality plugins.

The `mono/` fixtures include `.ayni.toml`. Use them with their own locked
managed environment when the fixture has the required native lock inputs, or
prepare the documented host prerequisites before using the explicit escape
hatch. For example, after installing the Go toolchain and `gocyclo`:

```sh
ayni check --host --config examples/go/mono/.ayni.toml
```

Pull-request CI builds the committed root environment, verifies it, and runs
Ayni's repository contract. Exercise affected fixtures locally through their
managed lifecycle when changing adapter or example behavior. Python pins its
runtime, uv version, and signal tools in `.python-version`, `pyproject.toml`, and
`uv.lock`; Kotlin pins its JDK contract, Gradle wrapper distribution, dependency
locks, and artifact verification metadata with the fixture.

The `single/` fixtures intentionally omit Ayni configuration so they can be
used as raw language examples. Use `ayni agents sync --repo-root <path>` only
when a fixture needs the managed guidance block.
