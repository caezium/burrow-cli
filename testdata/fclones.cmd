@echo off
if "%~1"=="group" (
  echo {"header":{"version":"0.35.0","stats":{"group_count":1,"total_file_count":2,"total_file_size":2048,"redundant_file_count":1,"redundant_file_size":1024}},"groups":[{"file_len":1024,"file_hash":"abc123","files":["/tmp/a","/tmp/b"]}]}
  exit /b 0
)
if "%~2"=="--dry-run" (
  more >NUL
  echo cp -c /tmp/a /tmp/b
  exit /b 0
)
if "%~1"=="dedupe" (
  more >NUL
  echo {"deduped":true,"action":"dedupe"}
  exit /b 0
)
if "%~1"=="remove" (
  more >NUL
  echo {"deduped":true,"action":"remove"}
  exit /b 0
)
if "%~1"=="link" (
  more >NUL
  echo {"deduped":true,"action":"link"}
  exit /b 0
)
echo fake fclones: unknown subcommand '%~1' 1>&2
exit /b 2
