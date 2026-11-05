using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class FieldGuiNative {
 public delegate bool EnumProc(IntPtr h, IntPtr p);
 [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb,IntPtr p);
 [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
 [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
 [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
[DllImport("user32.dll")] static extern bool IsZoomed(IntPtr h);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h,StringBuilder b,int n);
 [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l);
 [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
 public static long[] Windows(int pid) {var a=new List<long>();EnumWindows((h,p)=>{uint id;GetWindowThreadProcessId(h,out id);if(id==pid)a.Add(h.ToInt64());return true;},IntPtr.Zero);return a.ToArray();}
 public static string Title(long h){var b=new StringBuilder(1024);GetWindowText(new IntPtr(h),b,b.Capacity);return b.ToString();}
 public static void Close(long h,int pid){uint id;GetWindowThreadProcessId(new IntPtr(h),out id);if(id!=pid)throw new InvalidOperationException("Window identity changed: HWND="+h+", expectedPID="+pid+", actualPID="+id);if(!PostMessage(new IntPtr(h),0x10,IntPtr.Zero,IntPtr.Zero))throw new InvalidOperationException("WM_CLOSE failed");}
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h,StringBuilder b,int n);
  [DllImport("user32.dll")] static extern IntPtr SendMessageTimeout(IntPtr h,uint m,IntPtr w,IntPtr l,uint flags,uint timeout,out IntPtr result);
 [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
[DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
 static IntPtr ReadMenu(long h){IntPtr r;if(SendMessageTimeout(new IntPtr(h),0x1e1,IntPtr.Zero,IntPtr.Zero,2,1000,out r)==IntPtr.Zero)throw new InvalidOperationException("Native menu query timed out");return r;}
 [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr menu);

 [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetMenuString(IntPtr menu,uint item,StringBuilder text,int count,uint flags);
 public static string Class(long h){var b=new StringBuilder(256);GetClassName(new IntPtr(h),b,b.Capacity);return b.ToString();}
 public static void TrayMenu(int pid){foreach(var h in Windows(pid))if(Class(h)=="tray_icon_app"){Guard(h,pid);if(!PostMessage(new IntPtr(h),6002,IntPtr.Zero,new IntPtr(0x204)))throw new InvalidOperationException("Tray callback delivery failed");return;}throw new InvalidOperationException("Owned tray callback HWND missing");}
 static void Guard(long h,int pid){uint id;GetWindowThreadProcessId(new IntPtr(h),out id);if(id!=pid)throw new InvalidOperationException("Window identity changed: HWND="+h+", expectedPID="+pid+", actualPID="+id);}
 public static string[] MenuLabels(long h,int pid){Guard(h,pid);if(Class(h)!="#32768")throw new InvalidOperationException("Not a native menu window");var m=ReadMenu(h);int count=GetMenuItemCount(m);if(count<1||count>100)throw new InvalidOperationException("Invalid menu bound");var labels=new List<string>();for(uint i=0;i<count;i++){var b=new StringBuilder(512);GetMenuString(m,i,b,b.Capacity,0x400);labels.Add(b.ToString());}return labels.ToArray();}
 [StructLayout(LayoutKind.Sequential)] struct Rect {public int Left,Top,Right,Bottom;}
 [StructLayout(LayoutKind.Sequential)] struct Point {public int X,Y;}
 [DllImport("user32.dll")] static extern bool GetMenuItemRect(IntPtr h,IntPtr m,uint i,out Rect r);
 [DllImport("user32.dll")] static extern bool GetCursorPos(out Point p);
 [DllImport("user32.dll")] static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point p);
  [StructLayout(LayoutKind.Sequential)] struct MouseInput {public int X,Y;public uint Data,Flags,Time;public UIntPtr Extra;}
 [StructLayout(LayoutKind.Sequential)] struct Input {public uint Type;public MouseInput Mouse;}
 [DllImport("user32.dll",SetLastError=true)] static extern uint SendInput(uint count,Input[] input,int size);
 static void SendMouse(uint flags){var a=new[]{new Input{Type=0,Mouse=new MouseInput{Flags=flags}}};if(SendInput(1,a,Marshal.SizeOf(typeof(Input)))!=1)throw new InvalidOperationException("SendInput did not accept mouse event; Win32 error "+Marshal.GetLastWin32Error());}
 public static void SelectMenu(long h,int pid,string expected){var labels=MenuLabels(h,pid);int index=Array.IndexOf(labels,expected);if(index<0)throw new InvalidOperationException("Expected unique menu label missing");int matches=0;foreach(var x in labels)if(x==expected)matches++;if(matches!=1)throw new InvalidOperationException("Ambiguous menu label");Guard(h,pid);var menu=ReadMenu(h);Rect r;if(!GetMenuItemRect(IntPtr.Zero,menu,(uint)index,out r)||r.Right<=r.Left||r.Bottom<=r.Top)throw new InvalidOperationException("Menu rectangle unavailable");Point original;GetCursorPos(out original);var center=new Point{X=(r.Left+r.Right)/2,Y=(r.Top+r.Bottom)/2};try{if(!SetCursorPos(center.X,center.Y))throw new InvalidOperationException("SetCursorPos refused");System.Threading.Thread.Sleep(80);Point actual;if(!GetCursorPos(out actual)||actual.X!=center.X||actual.Y!=center.Y)throw new InvalidOperationException("Actual cursor did not reach selected owned menu rectangle");Guard(GetForegroundWindow().ToInt64(),pid);Guard(WindowFromPoint(center).ToInt64(),pid);if(Class(WindowFromPoint(center).ToInt64())!="#32768")throw new InvalidOperationException("Menu no longer under cursor");SendMouse(2);System.Threading.Thread.Sleep(100);SendMouse(4);System.Threading.Thread.Sleep(1000);}finally{SetCursorPos(original.X,original.Y);}}
 public static void ClickOwnedPoint(long h,int pid,int x,int y){Guard(h,pid);SetForegroundWindow(new IntPtr(h));System.Threading.Thread.Sleep(100);Guard(GetForegroundWindow().ToInt64(),pid);Point original;GetCursorPos(out original);var point=new Point{X=x,Y=y};try{if(!SetCursorPos(x,y))throw new InvalidOperationException("SetCursorPos refused");System.Threading.Thread.Sleep(100);Point actual;if(!GetCursorPos(out actual)||actual.X!=x||actual.Y!=y)throw new InvalidOperationException("Actual cursor did not reach selected owned control rectangle");Guard(GetForegroundWindow().ToInt64(),pid);Guard(WindowFromPoint(point).ToInt64(),pid);SendMouse(2);System.Threading.Thread.Sleep(100);SendMouse(4);System.Threading.Thread.Sleep(1000);}finally{SetCursorPos(original.X,original.Y);}}
 public static void TypeOwnedText(long h,int pid,string value){Guard(h,pid);Guard(GetForegroundWindow().ToInt64(),pid);if(value.Length>2048)throw new InvalidOperationException("Text bound exceeded");foreach(char c in value){Guard(h,pid);if(!PostMessage(new IntPtr(h),0x102,new IntPtr(c),IntPtr.Zero))throw new InvalidOperationException("WM_CHAR delivery failed");}}
 [DllImport("user32.dll")] static extern bool InvalidateRect(IntPtr h,IntPtr rect,bool erase);
 public static void RepaintOwned(long h,int pid){Guard(h,pid);InvalidateRect(new IntPtr(h),IntPtr.Zero,false);}
 [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h,out Rect r);
 public static int[] WindowBounds(long h,int pid){Guard(h,pid);Rect r;if(!GetWindowRect(new IntPtr(h),out r))throw new InvalidOperationException("Owned window bounds unavailable");return new[]{r.Left,r.Top,r.Right-r.Left,r.Bottom-r.Top};}
 public static void ClickObserved(long h,int pid,string title,int width,int height,int dpi,int x,int y){Guard(h,pid);var b=Metrics(h,pid);if(Title(h)!=title||b[10]!=width||b[11]!=height||GetDpiForWindow(new IntPtr(h))!=dpi||x<0||y<0||x>=width||y>=height)throw new InvalidOperationException("Observed CUA geometry/DPI/title contract mismatch");ClickOwnedPoint(h,pid,b[8]+x,b[9]+y);}
 [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h,out Rect r);
 [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr h,ref Point p);
 [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h,int attr,out Rect r,int size);
 [DllImport("user32.dll")] static extern IntPtr MonitorFromWindow(IntPtr h,uint flags);
 [StructLayout(LayoutKind.Sequential)] struct MonitorInfo {public int Size;public Rect Monitor,Work;public uint Flags;}
 [DllImport("user32.dll")] static extern bool GetMonitorInfo(IntPtr h,ref MonitorInfo m);
 public static int[] Metrics(long h,int pid){Guard(h,pid);var b=WindowBounds(h,pid);Rect c,d;Point p=new Point();if(!GetClientRect(new IntPtr(h),out c)||!ClientToScreen(new IntPtr(h),ref p)||DwmGetWindowAttribute(new IntPtr(h),9,out d,Marshal.SizeOf(typeof(Rect)))!=0)throw new InvalidOperationException("Window metrics unavailable");var m=new MonitorInfo{Size=Marshal.SizeOf(typeof(MonitorInfo))};if(!GetMonitorInfo(MonitorFromWindow(new IntPtr(h),2),ref m))throw new InvalidOperationException("Monitor metrics unavailable");return new[]{b[0],b[1],b[2],b[3],p.X,p.Y,c.Right-c.Left,c.Bottom-c.Top,d.Left,d.Top,d.Right-d.Left,d.Bottom-d.Top,m.Work.Left,m.Work.Top,m.Work.Right,m.Work.Bottom,(int)GetDpiForWindow(new IntPtr(h))};}
 public static void ClickObservedDownload(long h,int pid){var m=Metrics(h,pid);if(Title(h)!="ytTrayDL — download?"||m[10]!=432||(m[11]!=584&&m[11]!=546)||m[16]!=96)throw new InvalidOperationException("CUA observed download frame/title/DPI contract mismatch");if(m[8]<m[12]||m[9]<m[13]||m[8]+m[10]>m[14]||m[9]+m[11]>m[15])throw new InvalidOperationException("Download popup visible frame exceeds monitor workarea");ClickOwnedPoint(h,pid,m[8]+85,m[9]+m[11]-29);}
 [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int width,int height,uint flags);
 public static void MoveResizeOwned(long h,int pid,int x,int y,int width,int height){Guard(h,pid);if(IsIconic(new IntPtr(h))||IsZoomed(new IntPtr(h)))throw new InvalidOperationException("Persistence test requires normal window state");var m=Metrics(h,pid);if(x<m[12]||y<m[13]||x+width>m[14]||y+height>m[15]||width<340||height<420)throw new InvalidOperationException("Requested test rectangle outside selected workarea or below declared main minimum");if(!SetWindowPos(new IntPtr(h),IntPtr.Zero,x,y,width,height,0x14))throw new InvalidOperationException("Owned SetWindowPos failed");}
 [DllImport("user32.dll")] static extern uint GetMenuItemID(IntPtr menu,int position);
 [DllImport("user32.dll")] static extern uint GetMenuState(IntPtr menu,uint item,uint flags);
 public static uint DispatchObservedMenu(long h,int pid,string expected){var labels=MenuLabels(h,pid);int index=Array.IndexOf(labels,expected);if(index<0||Array.LastIndexOf(labels,expected)!=index)throw new InvalidOperationException("Native menu label missing or ambiguous");var menu=ReadMenu(h);if((GetMenuState(menu,(uint)index,0x400)&3)!=0)throw new InvalidOperationException("Native menu item disabled");uint id=GetMenuItemID(menu,index);if(id==0xffffffff||id>65535)throw new InvalidOperationException("Native menu command ID outside WM_COMMAND range");long owner=0;foreach(var candidate in Windows(pid))if(Class(candidate)=="tray_icon_app"){if(owner!=0)throw new InvalidOperationException("Tray owner ambiguous");owner=candidate;}if(owner==0)throw new InvalidOperationException("Tray owner missing");Guard(owner,pid);Guard(GetForegroundWindow().ToInt64(),pid);if(!PostMessage(new IntPtr(owner),0x1f,IntPtr.Zero,IntPtr.Zero)||!PostMessage(new IntPtr(owner),0x111,new IntPtr(id),IntPtr.Zero))throw new InvalidOperationException("Owned native menu command delivery failed");return id;}
 public static bool IsForegroundOwned(int pid){uint owner;GetWindowThreadProcessId(GetForegroundWindow(),out owner);return owner==pid;}
 public static bool FocusOwned(long h,int pid){Guard(h,pid);SetForegroundWindow(new IntPtr(h));System.Threading.Thread.Sleep(100);return IsForegroundOwned(pid);}
}
