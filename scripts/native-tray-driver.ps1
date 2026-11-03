# Native input/visibility evidence for one test-owned application only.
param([Parameter(Mandatory=$true)][int]$AppProcessId,
      [Parameter(Mandatory=$true)][string]$Exe,
      [Parameter(Mandatory=$true)][string]$StatePath,
      [Parameter(Mandatory=$true)][string]$ControlPath)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'installer-process.ps1')
$owned=Get-Process -Id $AppProcessId
if($owned.HasExited -or -not (Test-InstallerSameExecutable $owned.Path $Exe)){throw 'Test application ownership mismatch'}
$creation=$owned.StartTime.ToUniversalTime().ToFileTimeUtc()
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes,WindowsBase
Add-Type -ReferencedAssemblies @('System.Windows.Forms','System.Drawing','WindowsBase',[System.Windows.Automation.AutomationElement].Assembly.Location,[System.Windows.Automation.ControlType].Assembly.Location) -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
public static class TaskPulseTrayDriver {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int left,top,right,bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int x,y; }
    [StructLayout(LayoutKind.Sequential)] struct IconId { public uint size; public IntPtr window; public uint id; public Guid guid; }
    [StructLayout(LayoutKind.Sequential)] struct Mouse { public int x,y; public uint data,flags,time; public UIntPtr extra; }
    [StructLayout(LayoutKind.Explicit)] struct InputData { [FieldOffset(0)] public Mouse mouse; }
    [StructLayout(LayoutKind.Sequential)] struct Input { public uint type; public InputData data; }
    delegate bool Visit(IntPtr window,IntPtr data);
    delegate void Event(IntPtr hook,uint kind,IntPtr window,int objectId,int childId,uint thread,uint time);
    [DllImport("user32.dll")] static extern bool EnumWindows(Visit visit,IntPtr data);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window,out uint pid);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr window,StringBuilder text,int length);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr window,StringBuilder text,int length);
    [DllImport("user32.dll")] static extern bool PostMessageW(IntPtr window,uint message,UIntPtr param,IntPtr data);
    [DllImport("user32.dll")] static extern IntPtr SendMessageW(IntPtr window,uint message,IntPtr param,IntPtr data);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x,int y);
    [DllImport("user32.dll")] static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] static extern short GetAsyncKeyState(int key);
    [DllImport("kernel32.dll")] static extern uint WTSGetActiveConsoleSessionId();
    [DllImport("user32.dll")] static extern uint SendInput(uint count,Input[] input,int size);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window,int command);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr window,out Rect rect);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr window,IntPtr after,int x,int y,int width,int height,uint flags);
    [DllImport("user32.dll")] static extern IntPtr GetWindowLongPtrW(IntPtr window,int index);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetMenuStringW(IntPtr menu,uint item,StringBuilder text,int length,uint flags);
    [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr menu);
    [DllImport("user32.dll")] static extern bool GetMenuItemRect(IntPtr window,IntPtr menu,uint item,out Rect rect);
    [DllImport("shell32.dll")] static extern int Shell_NotifyIconGetRect(ref IconId id,out Rect rect);
    [DllImport("user32.dll")] static extern IntPtr SetWinEventHook(uint first,uint last,IntPtr module,Event callback,uint pid,uint thread,uint flags);
    [DllImport("user32.dll")] static extern bool UnhookWinEvent(IntPtr hook);
    [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] static extern IntPtr GetThreadDpiAwarenessContext();
    [DllImport("user32.dll")] static extern bool AreDpiAwarenessContextsEqual(IntPtr first,IntPtr second);
    static int pid;
    static Process process;
    static IntPtr tray,popup,hook;
    static Event callback;
    static IntPtr previousDpi;
    static Stopwatch clock=Stopwatch.StartNew();
    public static List<string> Events=new List<string>();
    public static bool Overflow;
    static bool held;
    public static bool PhysicalPointVerified;
    public static bool PerMonitorDpi;
    public static Point Target;
    public static Point Cursor(){Point point;GetCursorPos(out point);return point;}
    public static bool LeftDown(){return GetAsyncKeyState(1)<0;}
    public static void CheckHeldPointer(){if(held){var p=Cursor();if(Math.Abs(p.x-Target.x)>3||Math.Abs(p.y-Target.y)>3){ReleaseOwnedInput();throw new Exception("Native gesture interrupted by external pointer movement");}}}
    public static string InputLayout(){return Marshal.SizeOf(typeof(Input))+":"+Marshal.OffsetOf(typeof(Input),"data")+":"+Marshal.OffsetOf(typeof(Mouse),"flags");}
    public static int Session(){return Process.GetCurrentProcess().SessionId;}
    public static uint ConsoleSession(){return WTSGetActiveConsoleSessionId();}
    static void Check() {
        if(process.HasExited)throw new Exception("Owned process expired");
    }
    static bool Owned(IntPtr window) { uint owner; GetWindowThreadProcessId(window,out owner); return window!=IntPtr.Zero&&owner==(uint)pid; }
    static string Text(IntPtr window,bool name) { var s=new StringBuilder(512);if(name)GetClassNameW(window,s,s.Capacity);else GetWindowTextW(window,s,s.Capacity);return s.ToString(); }
    static IntPtr Find(string value,bool name) {
        IntPtr found=IntPtr.Zero;
        EnumWindows((w,d)=>{if(Owned(w)&&Text(w,name)==value){if(found!=IntPtr.Zero)throw new Exception("Ambiguous owned window");found=w;}return true;},IntPtr.Zero);
        return found;
    }
    public static void Initialize(int processId,long started) {
        previousDpi=SetThreadDpiAwarenessContext(new IntPtr(-4));
        PerMonitorDpi=AreDpiAwarenessContextsEqual(GetThreadDpiAwarenessContext(),new IntPtr(-4));
        if(!PerMonitorDpi)throw new Exception("Native geometry evidence requires per-monitor-v2 DPI awareness");
        pid=processId;process=Process.GetProcessById(pid);
        // Hold the process instance instead of reopening a PID on each sample.
        var handle=process.Handle;
        if(handle==IntPtr.Zero||process.StartTime.ToUniversalTime().ToFileTimeUtc()!=started)throw new Exception("Owned process identity changed");
        Check();tray=Find("tray_icon_app",true);
        if(tray==IntPtr.Zero)throw new Exception("Owned tray window missing");
        callback=(h,kind,w,obj,child,thread,time)=>{
            if(w!=popup||obj!=0||child!=0)return;
            if(Events.Count>=2048){Overflow=true;return;}
            Events.Add((kind==0x8002?"show:":"hide:")+clock.ElapsedMilliseconds);
        };
        hook=SetWinEventHook(0x8002,0x8003,IntPtr.Zero,callback,(uint)pid,0,0);
        if(hook==IntPtr.Zero)throw new Exception("Visibility event hook unavailable");
    }
    public static void Dispose(){ReleaseOwnedInput();if(hook!=IntPtr.Zero)UnhookWinEvent(hook);if(process!=null)process.Dispose();if(previousDpi!=IntPtr.Zero)SetThreadDpiAwarenessContext(previousDpi);}
    public static IntPtr Popup(){Check();if(!Owned(popup))popup=Find("TaskPulse Dev",false);return popup;}
    public static Rect Bounds(){Rect rect;if(!GetWindowRect(Popup(),out rect))throw new Exception("Owned popup bounds unavailable");return rect;}
    public static bool Visible(){return IsWindowVisible(Popup());}
    public static bool Focused(){return GetForegroundWindow()==Popup();}
    public static bool Topmost(){return (GetWindowLongPtrW(Popup(),-20).ToInt64()&8)!=0;}
    public static Rect TrayRect(){
        Check();if(!Owned(tray))throw new Exception("Owned tray expired");
        for(uint i=1;i<=16;i++){
            var id=new IconId{size=(uint)Marshal.SizeOf(typeof(IconId)),window=tray,id=i};Rect rect;
            if(Shell_NotifyIconGetRect(ref id,out rect)==0&&rect.right>rect.left&&rect.bottom>rect.top)return rect;
        }
        throw new Exception("Owned notification icon bounds unavailable");
    }
    public static void Notify(uint message){
        var rect=TrayRect();SetCursorPos((rect.left+rect.right)/2,(rect.top+rect.bottom)/2);
        PostTray(message);
    }
    static void PostTray(uint message){
        Check();if(!Owned(tray))throw new Exception("Owned tray expired");
        if(!PostMessageW(tray,6002,UIntPtr.Zero,new IntPtr(message)))throw new Exception("Owned tray callback failed");
    }
    static int roundsTarget,roundsStart,roundsPhase;
    static long roundsDeadline;
    public static int Rounds;
    public static void StartRounds(int count){
        if(count!=100||Visible())throw new Exception("Tray rounds require a hidden popup and exactly 100 iterations");
        roundsTarget=count;roundsStart=Events.Count;Rounds=0;roundsPhase=0;
        roundsDeadline=clock.ElapsedMilliseconds+90000;
    }
    public static void AdvanceRounds(){
        if(Rounds==roundsTarget)return;
        if(clock.ElapsedMilliseconds>roundsDeadline)throw new Exception("Tray rounds exceeded 90 seconds");
        int start=roundsStart+Rounds*2;
        if(Events.Count>start+2)throw new Exception("Unexpected extra visibility transition");
        if(roundsPhase==0){PostTray(0x201);roundsPhase=1;}
        else if(roundsPhase==1&&Events.Count>start&&Visible()){
            if(!Events[start].StartsWith("show:"))throw new Exception("Tray round did not open first");
            PostTray(0x202);PostTray(0x201);roundsPhase=2;
        }else if(roundsPhase==2&&Events.Count==start+2&&!Visible()){
            if(!Events[start+1].StartsWith("hide:"))throw new Exception("Tray round did not close second");
            PostTray(0x202);Rounds++;roundsPhase=0;
        }
    }
    public static void MouseButton(uint flag){var input=new Input{type=0,data=new InputData{mouse=new Mouse{flags=flag}}};if(SendInput(1,new[]{input},Marshal.SizeOf(typeof(Input)))!=1)throw new Exception("Native mouse input failed");if(flag==2)held=true;if(flag==4)held=false;}
    public static void ReleaseOwnedInput(){if(held)MouseButton(4);}
    public static void TrayPointer(){
        var r=TrayRect();int x=(r.left+r.right)/2,y=(r.top+r.bottom)/2;
        var element=System.Windows.Automation.AutomationElement.FromPoint(new System.Windows.Point(x,y));
        var b=element.Current.BoundingRectangle;
        if(!element.Current.Name.StartsWith("TaskPulse",StringComparison.Ordinal)||!b.Contains(x,y))throw new Exception("Owned tray icon is not physically accessible at its Shell rectangle");
        PhysicalPointVerified=true;Target=new Point{x=x,y=y};if(!SetCursorPos(x,y))throw new Exception("Owned tray cursor movement failed");
    }
    public static void ClickOwned(IntPtr window){Rect rect;if(!GetWindowRect(window,out rect)||!IsWindowVisible(window))throw new Exception("Fixture window unavailable");SetCursorPos((rect.left+rect.right)/2,rect.top+40);MouseButton(2);MouseButton(4);}
    public static string[] Menu(){
        Check();var window=Find("#32768",true);if(window==IntPtr.Zero)return new string[0];
        var menu=SendMessageW(window,0x1E1,IntPtr.Zero,IntPtr.Zero);
        var text=new List<string>();for(int i=0;i<GetMenuItemCount(menu);i++){var s=new StringBuilder(256);GetMenuStringW(menu,(uint)i,s,s.Capacity,0x400);text.Add(s.ToString());}return text.ToArray();
    }
    public static void SelectMenu(int index){
        Check();var window=Find("#32768",true);if(window==IntPtr.Zero)throw new Exception("Owned menu missing");
        var menu=SendMessageW(window,0x1E1,IntPtr.Zero,IntPtr.Zero);Rect rect;
        if(index<0||index>=GetMenuItemCount(menu)||!GetMenuItemRect(IntPtr.Zero,menu,(uint)index,out rect))throw new Exception("Owned menu item missing");
        SetCursorPos((rect.left+rect.right)/2,(rect.top+rect.bottom)/2);MouseButton(2);MouseButton(4);
    }
    public static void CancelMenu(){Check();var window=Find("#32768",true);if(window==IntPtr.Zero)throw new Exception("Owned menu missing");PostMessageW(window,0x100,new UIntPtr(27),IntPtr.Zero);}
    public static void Move(int monitor){
        Check();var screens=System.Windows.Forms.Screen.AllScreens;if(monitor<0||monitor>=screens.Length)throw new Exception("Monitor out of range");
        var r=screens[monitor].WorkingArea;
        if(!SetWindowPos(Popup(),IntPtr.Zero,r.Left+40,r.Top+40,0,0,0x15))throw new Exception("Owned popup move failed");
    }
}
'@
[TaskPulseTrayDriver]::Initialize($AppProcessId,$creation)
$form=New-Object Windows.Forms.Form
$form.Text='TaskPulse tray acceptance - test-owned, closes automatically'
$form.Width=440;$form.Height=180
$form.StartPosition='CenterScreen'
$form.Add_Shown({[void][TaskPulseTrayDriver]::ShowWindow($form.Handle,5)})
$form.Add_FormClosing({param($sender,$eventArgs)[Console]::Error.WriteLine('Fixture closing: '+$eventArgs.CloseReason)})
$script:requestId=0;$script:failure=$null
$timer=New-Object Windows.Forms.Timer;$timer.Interval=20
$timer.Add_Tick({
    try {
        [TaskPulseTrayDriver]::CheckHeldPointer()
        $request=$null
        if(Test-Path -LiteralPath $ControlPath){try{$request=[IO.File]::ReadAllText($ControlPath)|ConvertFrom-Json}catch{}}
        if($request -and $request.id -gt $script:requestId){
            switch($request.action){
                'notify-down' {[TaskPulseTrayDriver]::Notify(0x201)}
                'notify-up' {[TaskPulseTrayDriver]::Notify(0x202)}
                'notify-double' {[TaskPulseTrayDriver]::Notify(0x203)}
                'mouse-down' {[TaskPulseTrayDriver]::TrayPointer();[TaskPulseTrayDriver]::MouseButton(2)}
                'mouse-up' {[TaskPulseTrayDriver]::MouseButton(4)}
                'mouse-double' {[TaskPulseTrayDriver]::TrayPointer();[TaskPulseTrayDriver]::MouseButton(2);[TaskPulseTrayDriver]::MouseButton(4);[TaskPulseTrayDriver]::MouseButton(2);[TaskPulseTrayDriver]::MouseButton(4)}
                'release-input' {[TaskPulseTrayDriver]::ReleaseOwnedInput()}
                'blur' {[TaskPulseTrayDriver]::ClickOwned($form.Handle)}
                'menu' {[TaskPulseTrayDriver]::Notify(0x204);[TaskPulseTrayDriver]::Notify(0x205)}
                'menu-select' {[TaskPulseTrayDriver]::SelectMenu([int]$request.index)}
                'menu-cancel' {[TaskPulseTrayDriver]::CancelMenu()}
                'move' {[TaskPulseTrayDriver]::Move([int]$request.monitor)}
                'rounds' {[TaskPulseTrayDriver]::StartRounds(100)}
                default {throw 'Unknown tray fixture action'}
            }
            $script:requestId=$request.id
        }
        [TaskPulseTrayDriver]::AdvanceRounds()
        $rect=[TaskPulseTrayDriver]::Bounds()
        $state=@{request_id=$script:requestId;visible=[TaskPulseTrayDriver]::Visible();focused=[TaskPulseTrayDriver]::Focused();topmost=[TaskPulseTrayDriver]::Topmost();bounds=@{x=$rect.left;y=$rect.top;width=$rect.right-$rect.left;height=$rect.bottom-$rect.top};events=[TaskPulseTrayDriver]::Events.ToArray();overflow=[TaskPulseTrayDriver]::Overflow;menu=[TaskPulseTrayDriver]::Menu();physical_point_verified=[TaskPulseTrayDriver]::PhysicalPointVerified;per_monitor_dpi=[TaskPulseTrayDriver]::PerMonitorDpi}
        $state.rounds=[TaskPulseTrayDriver]::Rounds
        $state.input=@{layout=[TaskPulseTrayDriver]::InputLayout();left_down=[TaskPulseTrayDriver]::LeftDown();target=[TaskPulseTrayDriver]::Target;cursor=[TaskPulseTrayDriver]::Cursor();session=[TaskPulseTrayDriver]::Session();console_session=[TaskPulseTrayDriver]::ConsoleSession()}
        [IO.File]::WriteAllText($StatePath,($state|ConvertTo-Json -Depth 6 -Compress))
    } catch {
        $script:failure=$_.Exception.Message
        [IO.File]::WriteAllText($StatePath,(@{error=$script:failure}|ConvertTo-Json -Compress))
        $timer.Stop();$form.Close()
    }
})
try{$timer.Start();[Windows.Forms.Application]::Run($form)}
finally{$timer.Stop();$timer.Dispose();$form.Dispose();[TaskPulseTrayDriver]::Dispose();$owned.Dispose()}
if($script:failure){throw $script:failure}
