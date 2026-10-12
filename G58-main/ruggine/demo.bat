@echo off
set "TARGET_DIR=target\debug"

echo ---  1. Cleaning up old sessions... ---
:: Kill any running instances from previous tests
taskkill /F /IM gui_client_1.exe >nul 2>&1
taskkill /F /IM gui_client_2.exe >nul 2>&1
taskkill /F /IM server.exe >nul 2>&1
taskkill /F /IM client.exe >nul 2>&1

echo.
echo ---  2. Building Project... ---
cargo build
if %errorlevel% neq 0 (
    echo  Build failed! Fix errors before running.
    pause
    exit /b
)

echo.
echo ---  3. Creating "Shadow Copies" ---
:: Create copies so we can rebuild while running
copy /Y "%TARGET_DIR%\gui_client.exe" "%TARGET_DIR%\gui_client_1.exe" >nul
copy /Y "%TARGET_DIR%\gui_client.exe" "%TARGET_DIR%\gui_client_2.exe" >nul

echo.
echo --- 4. Launching Components ---

:: 1. Launch SERVER
:: REMOVED "cmd /k". Runs directly. Window closes when process dies.
start "Ruggine SERVER" "%TARGET_DIR%\server.exe"

:: Wait for server to initialize
timeout /t 2 /nobreak >nul

:: 2. Launch GUI CLIENTS
echo Starting GUIs...
start "GUI Client 1" "%TARGET_DIR%\gui_client_1.exe"
start "GUI Client 2" "%TARGET_DIR%\gui_client_2.exe"

:: 3. Launch CLI CLIENTS
:: REMOVED "cmd /k". Runs directly.
echo Starting CLIs...
start "CLI User A" "%TARGET_DIR%\client.exe"
start "CLI User B" "%TARGET_DIR%\client.exe"

echo.
echo ========================================================
echo   SYSTEM IS RUNNING
echo.
echo   [ PRESS ANY KEY ] to KILL ALL and CLOSE WINDOWS
echo ========================================================
pause >nul

echo.
echo --- CLOSING EVERYTHING... ---

:: Force kill the processes. 
:: Since we aren't using cmd /k, the windows will disappear instantly.
taskkill /F /IM server.exe >nul 2>&1
taskkill /F /IM client.exe >nul 2>&1
taskkill /F /IM gui_client_1.exe >nul 2>&1
taskkill /F /IM gui_client_2.exe >nul 2>&1

echo Done. Exiting...
timeout /t 1 >nul