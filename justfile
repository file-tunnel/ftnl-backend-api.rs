# Encrypted environment mechanics are delegated to ores-sops. Decrypted files
# live under ignored env/dec; env/enc contains reviewable SOPS ciphertext only.
set shell := ["bash", "-euo", "pipefail", "-c"]
set dotenv-load := false

_default:
    @just --list --unsorted

use name:
    @ores-sops use {{ name }}

status:
    @ores-sops status

edit name:
    @ores-sops edit {{ name }}

encrypt name:
    @ores-sops encrypt {{ name }}

diff name:
    @ores-sops diff {{ name }}

refresh:
    @ores-sops refresh

lock:
    @ores-sops lock

check:
    @agent-check
