# Quiet mode

The global `--quiet` flag suppresses informational output from successful
commands. It does not suppress errors, change exit codes, or skip work.

Plugins read the flag from `ForgeContext::quiet`. Warnings that tell a user
how to correct generated output are considered informational command output
and are suppressed along with success messages.

`--quiet` and `--verbose` may be used together: verbose logging remains
available on stderr while normal command output stays suppressed.
