# Encrypted environment bundles

Only `*.env.enc` SOPS ciphertext belongs here. Create or edit production data
with `nix develop --command just edit prod`; never redirect decrypted output
into this directory and never commit `env/dec`.

The bundle is intentionally not populated from credentials pasted into chat or
tickets. Rotate those values first, then enter the replacements directly in the
SOPS editor so plaintext does not enter shell history, patches, or CI logs.
