param([Parameter(Mandatory=$true)][string]$OutputPath)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
# Read only: capture only this build's visible TaskPulse icon, never the desktop
# or unrelated notification icons. No clicks, taskbar settings or DPI changes.
$captureRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\src-tauri\target'))
$capturePath=[IO.Path]::GetFullPath($OutputPath)
if (-not $capturePath.StartsWith($captureRoot+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) { throw 'Capture must stay under src-tauri/target.' }
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes,System.Drawing
Add-Type @'
using System.Runtime.InteropServices;
public static class TrayCaptureDpi {
  [DllImport("user32.dll")] public static extern uint GetDpiForSystem();
}
'@
$desktop=[System.Windows.Automation.AutomationElement]::RootElement
$shellClasses=@('Shell_TrayWnd','Shell_SecondaryTrayWnd','NotifyIconOverflowWindow','TopLevelWindowForOverflowXamlIsland')
$windows=$desktop.FindAll([System.Windows.Automation.TreeScope]::Children,[System.Windows.Automation.Condition]::TrueCondition)
$result=@{visible=$false;systemDpi=[TrayCaptureDpi]::GetDpiForSystem();reason='No visible matching TaskPulse tray icon; overflow/locked desktop not manipulated.'}
foreach($window in $windows) {
  if($window.Current.ClassName -notin $shellClasses){continue}
  $buttons=$window.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ControlTypeProperty,[System.Windows.Automation.ControlType]::Button))
  foreach($button in $buttons) {
    $name=$button.Current.Name
    # The new tooltip separates source faults / disconnected hosts with a slash.
    if($name -notlike 'TaskPulse*' -or $name -notlike '* / *' -or $button.Current.IsOffscreen){continue}
    $rect=$button.Current.BoundingRectangle
    if($rect.Width -lt 8 -or $rect.Height -lt 8 -or $rect.Width -gt 128 -or $rect.Height -gt 128){continue}
    $bitmap=[Drawing.Bitmap]::new([int]$rect.Width,[int]$rect.Height)
    $graphics=[Drawing.Graphics]::FromImage($bitmap)
    try {
      $graphics.CopyFromScreen([int]$rect.X,[int]$rect.Y,0,0,$bitmap.Size)
      $bitmap.Save($capturePath,[Drawing.Imaging.ImageFormat]::Png)
    } finally { $graphics.Dispose();$bitmap.Dispose() }
    $result=@{visible=$true;systemDpi=[TrayCaptureDpi]::GetDpiForSystem();tooltip=$name;width=$rect.Width;height=$rect.Height;path=$capturePath}
    break
  }
  if($result.visible){break}
}
$result | ConvertTo-Json -Compress
