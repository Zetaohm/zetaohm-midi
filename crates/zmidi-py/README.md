# zmidi-py

The native extension of the Python package [`zetaohm-midi`](../../bindings/python), built with
[PyO3](https://pyo3.rs). Python imports it as `zetaohm_midi._native`.

Part of [zetaohm-midi](../../README.md).

Do not use this crate directly: install or build the Python package. The crate is not a default
workspace member, because it needs a Python interpreter. `maturin` builds it from
[`bindings/python`](../../bindings/python), with the stable ABI for Python 3.9 and later
(`abi3-py39`).

```sh
cd bindings/python
uv sync          # builds this crate with maturin and installs the package
uv run pytest
```

The crate turns on Network MIDI 2.0 authentication. The Python Host takes its random nonce bytes
from `os.urandom` by default.

## License

Licensed under the [MIT license](LICENSE). Copyright (c) 2025-2026 Zetaohm LLC.
