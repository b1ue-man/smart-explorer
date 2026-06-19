# Repository Instructions

- After completing requested changes in this repository, commit the work and push it to the configured remote branch unless the user explicitly says not to or pushing is technically blocked.
- Do not leave completed work only in local commits; report the branch, commit, and push result.
- For native app changes, always bump the patch version, build the Windows release artifacts, commit the version/artifact changes on `main`, push `main`, create the matching `vX.Y.Z` tag, and push the tag unless the user explicitly says not to or this is technically blocked.
- For release work, always build the release artifacts before calling the release done, then commit and push the release changes and artifacts to the configured remote unless the user explicitly says not to or pushing is technically blocked.
- If a remote is missing, credentials fail, or the work is not safe to commit yet, state that clearly and explain what remains.
