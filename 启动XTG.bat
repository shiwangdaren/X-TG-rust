@echo off
chcp 65001 >nul
cd /d "%~dp0"

set "EXE=target\release\xtg-app.exe"

if exist "%EXE%" goto :run

echo [XTG] 未找到 Release 可执行文件，正在首次编译（仅需一次）...
where cargo >nul 2>&1
if errorlevel 1 (
  echo 错误: 未找到 cargo，请先安装 Rust 工具链 https://rustup.rs
  pause
  exit /b 1
)

cargo build --release -p xtg-app
if errorlevel 1 (
  echo 编译失败。
  pause
  exit /b 1
)

:run
rem start 的第一个引号参数是窗口标题，勿用空标题，否则在部分环境下路径解析异常
start "XTG" "%CD%\%EXE%"
