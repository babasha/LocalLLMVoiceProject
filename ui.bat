@echo off
REM Launch the desktop UI for the voice translator (lives in the system tray).
start "" powershell -NoProfile -ExecutionPolicy Bypass -STA -WindowStyle Hidden -File "%~dp0ui_win.ps1"
