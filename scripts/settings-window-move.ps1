param([int]$ProcessId,[string]$Exe,[string]$Title,[int]$X,[int]$Y)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'installer-process.ps1')
$process=Get-Process -Id $ProcessId
try {
    $null=$process.Handle
    if (-not (Test-InstallerSameExecutable $process.Path $Exe)) { throw 'Window owner executable mismatch' }
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class TaskPulseSettingsMove {
    delegate bool Callback(IntPtr hwnd,IntPtr unused);
    [DllImport("user32.dll")] static extern bool EnumWindows(Callback callback,IntPtr unused);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd,out uint pid);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr hwnd,StringBuilder text,int size);
    [DllImport("user32.dll",SetLastError=true)] static extern bool SetWindowPos(IntPtr hwnd,IntPtr after,int x,int y,int cx,int cy,uint flags);
    [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    public static void Move(uint pid,string title,int x,int y) {
        var matches=new List<IntPtr>();
        EnumWindows((hwnd,unused)=>{uint owner;GetWindowThreadProcessId(hwnd,out owner);var text=new StringBuilder(1024);GetWindowText(hwnd,text,text.Capacity);if(owner==pid&&text.ToString()==title)matches.Add(hwnd);return true;},IntPtr.Zero);
        if(matches.Count!=1)throw new Exception("Expected one owned settings window");
        uint currentOwner;GetWindowThreadProcessId(matches[0],out currentOwner);
        if(currentOwner!=pid)throw new Exception("Window ownership changed");
        var previous=SetThreadDpiAwarenessContext(new IntPtr(-4));
        try {if(!SetWindowPos(matches[0],IntPtr.Zero,x,y,0,0,0x0001|0x0004|0x0010))throw new System.ComponentModel.Win32Exception();}
        finally {if(previous!=IntPtr.Zero)SetThreadDpiAwarenessContext(previous);}
    }
}
'@
    if($process.HasExited){throw 'Owned app exited before movement'}
    [TaskPulseSettingsMove]::Move($ProcessId,$Title,$X,$Y)
} finally {$process.Dispose()}
