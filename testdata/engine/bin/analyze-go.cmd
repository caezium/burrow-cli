@echo off
echo %* | findstr /C:"--progress" >nul
if %errorlevel%==0 (
  echo {"type":"progress","files":1,"dirs":0,"bytes":4,"path":"/x"}
  echo {"type":"result","data":{"path":"/","overview":true,"entries":[],"total_size":0}}
) else (
  echo {"path":"/","overview":true,"entries":[],"total_size":0}
)
