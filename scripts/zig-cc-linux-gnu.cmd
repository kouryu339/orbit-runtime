@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0zig-cc-linux-gnu.ps1" %*
exit /b %ERRORLEVEL%