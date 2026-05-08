@echo off
setlocal enabledelayedexpansion

rem deecodex Windows 管理脚本
rem 用法: deecodex.bat {start|stop|restart|status|logs|health}

set "PROJECT_DIR=%~dp0"
set "ENV_FILE=%PROJECT_DIR%.env"
set "PID_FILE=%PROJECT_DIR%deecodex.pid"
set "LOG_DIR=%PROJECT_DIR%logs"
set "LOG_FILE=%LOG_DIR%deecodex.log"
set "BIN=deecodex.exe"
set "GRACEFUL_TIMEOUT=35"

if "%~1"=="" goto usage
goto case_%~1

:usage
echo 用法: %~nx0 {start^|stop^|restart^|status^|logs^|health}
exit /b 1

rem === 加载 .env ===
:load_env
if not exist "%ENV_FILE%" (
    echo 错误: 找不到 .env 文件 (%ENV_FILE%)
    echo       请先创建: copy .env.example .env && notepad .env
    exit /b 1
)
for /f "usebackq delims=" %%a in ("%ENV_FILE%") do (
    set "line=%%a"
    if not "!line!"=="" if not "!line:~0,1!"=="#" (
        for /f "tokens=1,2 delims==" %%b in ("!line!") do (
            set "%%b=%%c"
        )
    )
)
exit /b 0

rem === 环境变量映射 ===
:map_env
if "%DEECODEX_PORT%"=="" set "DEECODEX_PORT=4446"

set "CODEX_RELAY_UPSTREAM=%DEECODEX_UPSTREAM%"
set "CODEX_RELAY_API_KEY=%DEECODEX_API_KEY%"
set "CODEX_RELAY_PORT=%DEECODEX_PORT%"
set "CODEX_RELAY_MODEL_MAP=%DEECODEX_MODEL_MAP%"
exit /b 0

rem === 检查进程是否运行 ===
:is_running
if not exist "%PID_FILE%" exit /b 1
set /p PID=<"%PID_FILE%"
tasklist /fi "pid eq !PID!" 2>nul | find /i "!PID!" >nul
if errorlevel 1 exit /b 1
exit /b 0

rem === 日志轮转 ===
:rotate_logs
if not exist "%LOG_FILE%" exit /b 0
set /a MAX_BYTES=50*1024*1024
call :filesize "%LOG_FILE%" SIZE
if %SIZE% lss %MAX_BYTES% exit /b 0
if exist "%LOG_FILE%.5" del "%LOG_FILE%.5"
for /l %%i in (4,-1,1) do (
    if exist "%LOG_FILE%.%%i" ren "%LOG_FILE%.%%i" "%LOG_FILE%.%%~ni.%%~xi"
)
ren "%LOG_FILE%" "%LOG_FILE%.1"
type nul > "%LOG_FILE%"
exit /b 0

:filesize
set "%~2=%~z1"
exit /b 0

rem === Codex 配置管理 ===
:codex_config_init
set "CODEX_CONFIG=%USERPROFILE%\.codex\config.toml"
if not exist "%CODEX_CONFIG%" exit /b 0

set "CODEX_CONFIG_OPENAI=%CODEX_CONFIG%.openai.txt"
set "CODEX_CONFIG_DEECODEX=%CODEX_CONFIG%.deecodex.txt"

if not exist "%CODEX_CONFIG_OPENAI%" (
    copy /y "%CODEX_CONFIG%" "%CODEX_CONFIG_OPENAI%" >nul
    echo 已备份 Codex 配置
)

rem 生成 deecodex 配置
copy /y "%CODEX_CONFIG_OPENAI%" "%CODEX_CONFIG_DEECODEX%" >nul
(
echo.
echo # === 以下由 deecodex 自动管理 ===
echo [model_providers.custom]
echo base_url = "http://127.0.0.1:%DEECODEX_PORT%/v1"
echo name = "custom"
echo requires_openai_auth = false
echo wire_api = "responses"
) >> "%CODEX_CONFIG_DEECODEX%"

echo 已生成 deecodex 配置 (端口: %DEECODEX_PORT%)
exit /b 0

:codex_config_switch_to_deecodex
if not exist "%CODEX_CONFIG_DEECODEX%" exit /b 0
copy /y "%CODEX_CONFIG_DEECODEX%" "%CODEX_CONFIG%" >nul
exit /b 0

:codex_config_switch_to_openai
if not exist "%CODEX_CONFIG_OPENAI%" exit /b 0
copy /y "%CODEX_CONFIG_OPENAI%" "%CODEX_CONFIG%" >nul
exit /b 0

rem === start ===
:case_start
call :is_running 2>nul
if not errorlevel 1 (
    set /p RPID=<"%PID_FILE%"
    echo deecodex 已在运行中 (PID: !RPID!)
    exit /b 1
)

where %BIN% >nul 2>&1
if errorlevel 1 (
    echo 错误: 找不到 %BIN%，请确认已安装到 PATH
    echo       下载: https://github.com/liguan-89/deecodex/releases
    exit /b 1
)

call :load_env
if errorlevel 1 exit /b 1
call :map_env

if "%DEECODEX_API_KEY%"=="" (
    echo 错误: 请在 .env 中填入 DEECODEX_API_KEY
    exit /b 1
)

if not exist "%LOG_DIR%" mkdir "%LOG_DIR%"
call :codex_config_init
call :codex_config_switch_to_deecodex
call :rotate_logs

echo 启动 deecodex (端口: %DEECODEX_PORT%)...
start /b "" "%BIN%" --port %DEECODEX_PORT% --upstream %DEECODEX_UPSTREAM% --model-map "%DEECODEX_MODEL_MAP%" >> "%LOG_FILE%" 2>&1

rem 获取启动进程 PID
set PID=
for /f "tokens=2" %%a in ('tasklist /fi "imagename eq %BIN%" /fo list 2^>nul ^| find "PID:"') do set PID=%%a
echo !PID! > "%PID_FILE%"

timeout /t 2 /nobreak >nul
echo deecodex 已启动 (PID: !PID!, 端口: %DEECODEX_PORT%)
exit /b 0

rem === stop ===
:case_stop
call :is_running 2>nul
if errorlevel 1 (
    echo deecodex 未运行
    call :codex_config_switch_to_openai
    del "%PID_FILE%" 2>nul
    exit /b 0
)

set /p PID=<"%PID_FILE%"
echo 停止 deecodex (PID: %PID%)...
taskkill /pid %PID% >nul 2>&1

set /a waited=0
:wait_loop
timeout /t 1 /nobreak >nul
set /a waited+=1
tasklist /fi "pid eq %PID%" 2>nul | find /i "%PID%" >nul
if errorlevel 1 (
    echo 已停止 (!waited!s)
    call :codex_config_switch_to_openai
    del "%PID_FILE%" 2>nul
    exit /b 0
)
if %waited% lss %GRACEFUL_TIMEOUT% goto wait_loop

echo 优雅退出超时，强制终止...
taskkill /f /pid %PID% >nul 2>&1
call :codex_config_switch_to_openai
del "%PID_FILE%" 2>nul
echo 已强制停止
exit /b 0

rem === restart ===
:case_restart
call :case_stop
timeout /t 1 /nobreak >nul
call :case_start
exit /b 0

rem === status ===
:case_status
call :is_running 2>nul
if errorlevel 1 (
    echo deecodex 未运行
    del "%PID_FILE%" 2>nul
    exit /b 0
)
set /p PID=<"%PID_FILE%"
echo deecodex 运行中
echo   PID:    %PID%
echo   端口:   %DEECODEX_PORT%
echo   日志:   %LOG_FILE%
exit /b 0

rem === logs ===
:case_logs
if exist "%LOG_FILE%" (
    type "%LOG_FILE%"
    echo 实时日志请用: Get-Content "%LOG_FILE%" -Wait
) else (
    echo 暂无日志 (%LOG_FILE%)
)
exit /b 0

rem === health ===
:case_health
call :load_env 2>nul
call :map_env
if "%DEECODEX_PORT%"=="" set "DEECODEX_PORT=4446"

curl -s -o nul -w "%%{http_code}" http://127.0.0.1:%DEECODEX_PORT%/v1/models >nul 2>&1
if %errorlevel% neq 0 (
    echo unreachable (端口 %DEECODEX_PORT% 无响应，请先 deecodex.bat start)
    exit /b 0
)

for /f %%a in ('curl -s -o nul -w "%%{http_code}" http://127.0.0.1:%DEECODEX_PORT%/v1/models 2^>nul') do set CODE=%%a
if "%CODE%"=="200" (
    echo healthy (GET /v1/models → %CODE%)
) else (
    echo degraded (GET /v1/models → %CODE%)
)
exit /b 0

endlocal
