# Historical public history handoff — nextcore-core

## Current Status

The authoritative main at `92fcc9c9b500cd15f89df34a85cd78459801013c` is the active module API. The older local public main `c08d10b955523dd5da5a04084d6ee1c332760413` and frozen source HEAD `147f4c4e4969074c9dd12c63bf17c6e8c674dabb` contain 6 commit identities absent from that initial main ancestry. Ordinary local merge commits preserve those identities without replacing the current API or dependency pins. Publication must be verified separately.

## Target State

Keep the current module implementation active. Retain the distinct older conflict variants below as historical research source with their exact bytes, original paths, Git blob identifiers and SHA-256 hashes in [manifest.json](manifest.json). The preserved variants are unfinished historical work; their presence makes no compiler, runtime, operating-system boot, Metal or device acceptance claim.

## Conflict disposition

The history merge has no active source tree changes. The missing identities are already superseded in the current source tree.

## License

Each recorded source revision includes its original `LICENSE.txt` in the manifest. The current module [LICENSE.txt](../../../LICENSE.txt) also remains unchanged.
