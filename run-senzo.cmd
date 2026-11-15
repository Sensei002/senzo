@echo off
setlocal
set "SENZO_DIR=%~dp0dist\SENZO"
if not exist "%SENZO_DIR%\senzo.exe" (
    echo [SENZO] Release package not found at:
    echo   %SENZO_DIR%\senzo.exe
    echo.
    echo Please download or build the release package first, or run:
    echo   gh run download --dir dist
    echo.
    pause
    exit /b 1
)
echo [SENZO] Launching SENZO native media center...
start "" "%SENZO_DIR%\senzo.exe"
