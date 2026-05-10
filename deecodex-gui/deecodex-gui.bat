@echo off
setlocal
set "INSTALL_DIR=%LOCALAPPDATA%\Programs\deecodex"
if exist "%INSTALL_DIR%\deecodex-gui.exe" (
    start "" "%INSTALL_DIR%\deecodex-gui.exe"
) else (
    echo deecodex-gui.exe 未找到，请确认已安装
    pause
)
