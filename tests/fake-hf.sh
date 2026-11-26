#!/bin/sh
# fake-hf download <repo> <file> --local-dir <dir>
mkdir -p "$(dirname "$5/$3")" && printf '12345' > "$5/$3"
