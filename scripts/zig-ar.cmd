@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0zig-ar.ps1" %*
exit /b %ERRORLEVEL%