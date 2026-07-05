@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0zig-cxx-linux-gnu.ps1" %*
exit /b %ERRORLEVEL%