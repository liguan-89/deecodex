@echo off
chcp 65001 >nul 2>&1

setlocal enabledelayedexpansion

rem deecodex Windows �����ű�
rem �÷�: deecodex.bat {start|stop|restart|status|logs|health|update}

set "PROJECT_DIR=%~dp0"
set "ENV_FILE=%PROJECT_DIR%.env"
set "PID_FILE=%PROJECT_DIR%deecodex.pid"
set "LOG_DIR=%PROJECT_DIR%logs"
set "LOG_FILE=%LOG_DIR%deecodex.log"
set "BIN=deecodex.exe"
set "GRACEFUL_TIMEOUT=35"

set "GH_REPO=liguan-89/deecodex"

if "%~1"=="" goto menu
goto case_%~1

:menu
cls
echo ================================
echo   deecodex �����˵�
echo ================================
echo.
echo   [1] ��������
echo   [2] ֹͣ����
echo   [3] ��������
echo   [4] �鿴״̬
echo   [5] �������
echo   [6] �鿴��־
echo   [7] �������°�
echo   [0] �˳�
echo.
set /p CHOICE="��ѡ�� (0-7): "
if "%CHOICE%"=="1" goto case_start
if "%CHOICE%"=="2" goto case_stop
if "%CHOICE%"=="3" goto case_restart
if "%CHOICE%"=="4" goto case_status
if "%CHOICE%"=="5" goto case_health
if "%CHOICE%"=="6" goto case_logs
if "%CHOICE%"=="7" goto case_update
if "%CHOICE%"=="0" exit /b 0
goto menu

:usage
echo �÷�: %~nx0 {start^|stop^|restart^|status^|logs^|health^|update}
echo       ֱ��˫�����пɽ��뽻���˵�
exit /b 1

rem === ���� .env ===
:load_env
if not exist "%ENV_FILE%" (
    echo ����: �Ҳ��� .env �ļ� [%ENV_FILE%]
    echo       �뽫 .env.example ������Ϊ .env ���ü��±����� API Key
    pause
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

rem === ��������ӳ�� ===
:map_env
if "%DEECODEX_PORT%"=="" set "DEECODEX_PORT=4446"

set "CODEX_RELAY_UPSTREAM=%DEECODEX_UPSTREAM%"
set "CODEX_RELAY_API_KEY=%DEECODEX_API_KEY%"
set "CODEX_RELAY_PORT=%DEECODEX_PORT%"
if not "%DEECODEX_MODEL_MAP%"=="" set "CODEX_RELAY_MODEL_MAP=%DEECODEX_MODEL_MAP%"
exit /b 0

rem === �������Ƿ����� ===
:is_running
if not exist "%PID_FILE%" goto pid_fallback
set /p PID=<"%PID_FILE%"
tasklist /fi "pid eq !PID!" 2>nul | find /i "!PID!" >nul
if not errorlevel 1 exit /b 0
rem PID �ļ����ڣ�����
del "%PID_FILE%" 2>nul
:pid_fallback
rem ���ˣ�ͨ������������
for /f "tokens=2" %%a in ('tasklist /fi "imagename eq %BIN%" /fo list 2^>nul ^| find "PID:"') do (
    echo %%a > "%PID_FILE%"
    exit /b 0
)
exit /b 1

rem === ��־��ת ===
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

rem === Codex ���ù��� ===
rem === Codex ��� ===
:detect_codex
rem ���� 0=�Ѱ�װ, 1=δ��װ
rem 1. ~/.codex Ŀ¼����
if exist "%USERPROFILE%\.codex" exit /b 0
rem 2. codex �� PATH ��
where codex >nul 2>&1
if not errorlevel 1 exit /b 0
rem 3. �����/MSI ��װ
if exist "%LOCALAPPDATA%\Programs\codex" exit /b 0
rem 4. Microsoft Store �汾
if exist "C:\Program Files\WindowsApps" (
    for /d %%d in ("C:\Program Files\WindowsApps\OpenAI.Codex*") do exit /b 0
)
exit /b 1

rem === Codex ���ù��� ===
:codex_config_init
set "CODEX_CONFIG=%USERPROFILE%\.codex\config.toml"
set "CODEX_DIR=%USERPROFILE%\.codex"

if not exist "%CODEX_CONFIG%" (
    call :detect_codex 2>nul
    if errorlevel 1 exit /b 0
    rem Codex �Ѱ�װ�� config.toml ��δ������������״�ʹ�ã�
    if not exist "%CODEX_DIR%" mkdir "%CODEX_DIR%"
    type nul > "%CODEX_CONFIG%"
)

set "CODEX_CONFIG_OPENAI=%CODEX_CONFIG%.openai.txt"
set "CODEX_CONFIG_DEECODEX=%CODEX_CONFIG%.deecodex.txt"

if not exist "%CODEX_CONFIG_OPENAI%" (
    copy /y "%CODEX_CONFIG%" "%CODEX_CONFIG_OPENAI%" >nul
    echo �ѱ��� Codex ����
)

rem ���� deecodex ����
copy /y "%CODEX_CONFIG_OPENAI%" "%CODEX_CONFIG_DEECODEX%" >nul
rem remove existing [model_providers.custom] section
powershell -NoProfile -Command "$skip=$false; $lines=@(); foreach ($line in Get-Content '%CODEX_CONFIG_DEECODEX%') { if ($line -match '^\[model_providers\.custom\]') { $skip=$true; continue } if ($skip -and $line -match '^\[') { $skip=$false } if (-not $skip) { $lines += $line } }; $lines | Set-Content '%CODEX_CONFIG_DEECODEX%' -Encoding UTF8"
(
echo.
echo # === ������ deecodex �Զ����� ===
echo model_provider = "custom"
echo.
echo [model_providers.custom]
echo base_url = "http://127.0.0.1:!DEECODEX_PORT!/v1"
echo name = "custom"
echo requires_openai_auth = false
echo wire_api = "responses"
) >> "%CODEX_CONFIG_DEECODEX%"

echo ������ deecodex ���� (�˿�: !DEECODEX_PORT!)
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
    echo deecodex ���������� [PID: !RPID!]
    exit /b 1
)

rem ���ȼ��ű�ͬĿ¼����� PATH��֧�ֱ�Я�ⰲװ��
set "BIN_PATH="
if exist "%PROJECT_DIR%%BIN%" (
    set "BIN_PATH=%PROJECT_DIR%%BIN%"
) else (
    where %BIN% >nul 2>&1
    if not errorlevel 1 set "BIN_PATH=%BIN%"
)
if "%BIN_PATH%"=="" (
    echo ����: �Ҳ��� %BIN%���뽫 %BIN% ���ڽű�ͬĿ¼
    echo       ����: https://github.com/liguan-89/deecodex/releases
    pause
    exit /b 1
)

call :load_env
if errorlevel 1 exit /b 1
call :map_env

if "%DEECODEX_API_KEY%"=="" (
    echo ����: ���� .env ������ DEECODEX_API_KEY
    echo       �ü��±��� %ENV_FILE% �޸�
    pause
    exit /b 1
)

if not exist "%LOG_DIR%" mkdir "%LOG_DIR%"
rem 在注入前修复已知的 Codex config.toml 错误（打破污染循环）
"%BIN_PATH%" fix-config 2>nul
call :codex_config_init
call :codex_config_switch_to_deecodex
call :rotate_logs

echo ���� deecodex (�˿�: !DEECODEX_PORT!)...
rem model-map is read by binary from env DEECODEX_MODEL_MAP
rem (removed redundant --model-map CLI arg)
start /b "" "!BIN_PATH!" --port !DEECODEX_PORT! --upstream !DEECODEX_UPSTREAM! >> "%LOG_FILE%" 2>&1

rem ��ȡ�������� PID
set PID=
for /f "tokens=2" %%a in ('tasklist /fi "imagename eq %BIN%" /fo list 2^>nul ^| find "PID:"') do set PID=%%a
echo !PID! > "%PID_FILE%"

timeout /t 2 /nobreak >nul
echo deecodex ������ (PID: !PID!, �˿�: !DEECODEX_PORT!)
exit /b 0

rem === stop ===
:case_stop
call :is_running 2>nul
if errorlevel 1 (
    echo deecodex δ����
    call :codex_config_switch_to_openai
    del "%PID_FILE%" 2>nul
    exit /b 0
)

set /p PID=<"%PID_FILE%"
echo ֹͣ deecodex (PID: %PID%)...
taskkill /pid %PID% >nul 2>&1

set /a waited=0
:wait_loop
timeout /t 1 /nobreak >nul
set /a waited+=1
tasklist /fi "pid eq %PID%" 2>nul | find /i "%PID%" >nul
if errorlevel 1 (
    echo ��ֹͣ (!waited!s)
    call :codex_config_switch_to_openai
    del "%PID_FILE%" 2>nul
    exit /b 0
)
if %waited% lss %GRACEFUL_TIMEOUT% goto wait_loop

echo �����˳���ʱ��ǿ����ֹ...
taskkill /f /pid %PID% >nul 2>&1
call :codex_config_switch_to_openai
del "%PID_FILE%" 2>nul
echo ��ǿ��ֹͣ
exit /b 0

rem === restart ===
:case_restart
call :case_stop
timeout /t 1 /nobreak >nul
call :case_start
exit /b 0

rem === status ===
:case_status
call :load_env 2>nul
call :map_env
call :is_running 2>nul
if errorlevel 1 (
    echo deecodex δ����
    del "%PID_FILE%" 2>nul
    exit /b 0
)
set /p PID=<"%PID_FILE%"
echo deecodex ������
echo   PID:    !PID!
echo   �˿�:   !DEECODEX_PORT!
echo   ��־:   %LOG_FILE%
exit /b 0

rem === logs ===
:case_logs
if exist "%LOG_FILE%" (
    type "%LOG_FILE%"
    echo ʵʱ��־����: Get-Content "%LOG_FILE%" -Wait
) else (
    echo ������־ [%LOG_FILE%]
)
exit /b 0

rem === health ===
:case_health
call :load_env 2>nul
call :map_env
if "%DEECODEX_PORT%"=="" set "DEECODEX_PORT=4446"

curl -s -o nul -w "%%{http_code}" http://127.0.0.1:%DEECODEX_PORT%/v1/models >nul 2>&1
if %errorlevel% neq 0 (
    echo unreachable [�˿� !DEECODEX_PORT! ����Ӧ������ deecodex.bat start]
    exit /b 0
)

for /f %%a in ('curl -s -o nul -w "%%{http_code}" http://127.0.0.1:%DEECODEX_PORT%/v1/models 2^>nul') do set CODE=%%a
if "%CODE%"=="200" (
    echo healthy [GET /v1/models �� %CODE%]
) else (
    echo degraded [GET /v1/models �� %CODE%]
)
exit /b 0

rem === update ===
:case_update
echo ������°汾...
for /f "delims=" %%a in ('curl -sS "https://api.github.com/repos/%GH_REPO%/releases/latest" 2^>nul ^| findstr /r """tag_name"""') do set TAG_LINE=%%a
if "%TAG_LINE%"=="" (
    echo ����: �޷���ȡ���°汾
    exit /b 1
)
for /f "tokens=2 delims=:" %%a in ("%TAG_LINE%") do set TAG=%%~a
set TAG=!TAG: =!
set TAG=!TAG:"=!
set TAG=!TAG:,=!
echo ���°汾: !TAG!

set TEMP_DIR=%TEMP%\deecodex_update
if exist "%TEMP_DIR%" rmdir /s /q "%TEMP_DIR%"
mkdir "%TEMP_DIR%"

echo ���� deecodex.exe (!TAG!)...
curl -fsSL "https://github.com/%GH_REPO%/releases/download/!TAG!/deecodex.exe" -o "%TEMP_DIR%\deecodex.exe"
if not exist "%TEMP_DIR%\deecodex.exe" (
    echo ����: ����ʧ��
    exit /b 1
)

set WAS_RUNNING=0
call :is_running 2>nul
if not errorlevel 1 set WAS_RUNNING=1

if !WAS_RUNNING! equ 1 (
    echo ֹͣ�ɰ汾...
    call :case_stop
)

echo �滻������...
move /y "%TEMP_DIR%\deecodex.exe" "%PROJECT_DIR%deecodex.exe" >nul
echo �Ѹ���: %PROJECT_DIR%deecodex.exe

echo ���¹����ű�...
curl -fsSL "https://github.com/%GH_REPO%/releases/download/!TAG!/deecodex.bat" -o "%TEMP_DIR%\deecodex.bat"
if exist "%TEMP_DIR%\deecodex.bat" (
    move /y "%TEMP_DIR%\deecodex.bat" "%PROJECT_DIR%deecodex.bat" >nul
    echo �Ѹ���: %PROJECT_DIR%deecodex.bat
)


echo �о�����Ӧ��...
curl -fsSL "https://github.com/%GH_REPO%/releases/download/!TAG!/deecodex-gui.exe" -o "%TEMP_DIR%\deecodex-gui.exe"
if exist "%TEMP_DIR%\deecodex-gui.exe" (
    move /y "%TEMP_DIR%\deecodex-gui.exe" "%PROJECT_DIR%deecodex-gui.exe" >nul
    echo �Ѹ���: %PROJECT_DIR%deecodex-gui.exe
)

echo ����������...
curl -fsSL "https://github.com/%GH_REPO%/releases/download/!TAG!/WebView2Loader.dll" -o "%TEMP_DIR%\WebView2Loader.dll"
if exist "%TEMP_DIR%\WebView2Loader.dll" (
    move /y "%TEMP_DIR%\WebView2Loader.dll" "%PROJECT_DIR%WebView2Loader.dll" >nul
    echo �Ѹ���: %PROJECT_DIR%WebView2Loader.dll
)
curl -fsSL "https://github.com/%GH_REPO%/releases/download/!TAG!/icon.ico" -o "%TEMP_DIR%\icon.ico"
if exist "%TEMP_DIR%\icon.ico" (
    move /y "%TEMP_DIR%\icon.ico" "%PROJECT_DIR%icon.ico" >nul
    echo �Ѹ���: %PROJECT_DIR%icon.ico
)
rem sync .env.example when missing
if not exist "%ENV_FILE%" (
    curl -fsSL "https://github.com/%GH_REPO%/releases/download/!TAG!/env.example" -o "%PROJECT_DIR%\.env.example"
)
rmdir /s /q "%TEMP_DIR%" 2>nul

if !WAS_RUNNING! equ 1 (
    echo ��������...
    call :case_start
)

echo ������� (!TAG!)
exit /b 0

endlocal
