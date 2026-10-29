# Tech debt

Bugs and shortcuts found after milestone 1, newest first. Features go to [BACKLOG.md](BACKLOG.md).

| Issue | Where | Notes |
| --- | --- | --- |
| Token pull not checked against GitHub or from the Finder-started bundle | accounts, `make build` | Same code path as the GitLab check; run once with a GitHub PAT. |
