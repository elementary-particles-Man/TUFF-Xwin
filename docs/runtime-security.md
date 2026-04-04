# Runtime Security Guidelines

## Target Directory
The TUFF-Xwin architecture utilizes a runtime directory to store IPC sockets, scene snapshots, and operational metadata.

- **Recommendation:** Always use `XDG_RUNTIME_DIR` (e.g., `/run/user/1000`) for secure, user-bound runtime artifact storage.
- **Shared Environments:** When operating in a shared temporary environment (e.g., fallback to `/tmp`), an explicitly designated and isolated runtime directory must be used.

## Artifact Sensitivity
The runtime artifacts may contain sensitive operational UI metadata, window hierarchies, and clipboard serial identifiers (though the content itself is not persisted). 
It is crucial that these artifacts are not exposed to unauthorized users on the same system.

## Access Permissions
When the system creates a new runtime directory, its permissions should ideally be restricted to the owner (equivalent to `0700`). 
However, due to OS and environment dependencies, **it is the responsibility of the operator or the user session initialization scripts to ensure that `XDG_RUNTIME_DIR` and its subdirectories are appropriately permissioned (`0700`)** to prevent cross-user data exposure.
