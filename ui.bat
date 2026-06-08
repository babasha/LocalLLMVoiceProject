@echo off
REM Launch the lightweight desktop UI for the voice translator.
powershell -NoProfile -ExecutionPolicy Bypass -STA -File "%~dp0ui_win.ps1"
