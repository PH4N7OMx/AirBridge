"""Display a generated H.264 fixture through the real renderer and capture QA images."""
import ctypes
from ctypes import wintypes
from pathlib import Path
import subprocess
import os
import tempfile
import time
import winreg
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
QA = Path(__file__).resolve().parent / 'tools/ui-qa'
QA.mkdir(parents=True, exist_ok=True)
user = ctypes.WinDLL('user32', use_last_error=True)
user.FindWindowW.argtypes = [wintypes.LPCWSTR,wintypes.LPCWSTR]
user.FindWindowW.restype = wintypes.HWND
user.GetWindowRect.argtypes = [wintypes.HWND,ctypes.POINTER(wintypes.RECT)]
user.PostMessageW.argtypes = [wintypes.HWND,wintypes.UINT,wintypes.WPARAM,wintypes.LPARAM]
user.SetForegroundWindow.argtypes = [wintypes.HWND]
user.SetWindowPos.argtypes = [wintypes.HWND,wintypes.HWND,ctypes.c_int,ctypes.c_int,ctypes.c_int,ctypes.c_int,wintypes.UINT]
user.GetDlgItem.argtypes = [wintypes.HWND,ctypes.c_int]
user.GetDlgItem.restype = wintypes.HWND
user.SendMessageW.argtypes = [wintypes.HWND,wintypes.UINT,wintypes.WPARAM,wintypes.LPARAM]
user.SendMessageW.restype = wintypes.LPARAM
user.IsWindowEnabled.argtypes = [wintypes.HWND]
user.IsWindowVisible.argtypes = [wintypes.HWND]
user.GetClientRect.argtypes = [wintypes.HWND,ctypes.POINTER(wintypes.RECT)]
user.PrintWindow.argtypes = [wintypes.HWND,wintypes.HDC,wintypes.UINT]
user.GetMenu.argtypes = [wintypes.HWND]
user.GetMenu.restype = wintypes.HMENU
user.GetSubMenu.argtypes = [wintypes.HMENU,ctypes.c_int]
user.GetSubMenu.restype = wintypes.HMENU
user.GetMenuState.argtypes = [wintypes.HMENU,wintypes.UINT,wintypes.UINT]
user.GetDC.argtypes = [wintypes.HWND]
user.GetDC.restype = wintypes.HDC
user.ReleaseDC.argtypes = [wintypes.HWND,wintypes.HDC]
user.GetWindowTextW.argtypes = [wintypes.HWND,wintypes.LPWSTR,ctypes.c_int]
user.SetWindowTextW.argtypes = [wintypes.HWND,wintypes.LPCWSTR]
user.GetWindowDC.argtypes = [wintypes.HWND]
user.GetWindowDC.restype = wintypes.HDC
user.GetWindowLongPtrW.argtypes = [wintypes.HWND,ctypes.c_int]
user.GetWindowLongPtrW.restype = ctypes.c_ssize_t
class GuiThreadInfo(ctypes.Structure):
    _fields_=[('cbSize',wintypes.DWORD),('flags',wintypes.DWORD),
              ('hwndActive',wintypes.HWND),('hwndFocus',wintypes.HWND),('hwndCapture',wintypes.HWND),
              ('hwndMenuOwner',wintypes.HWND),('hwndMoveSize',wintypes.HWND),('hwndCaret',wintypes.HWND),
              ('rcCaret',wintypes.RECT)]
user.GetGUIThreadInfo.argtypes=[wintypes.DWORD,ctypes.POINTER(GuiThreadInfo)]
user.GetWindowThreadProcessId.argtypes=[wintypes.HWND,ctypes.POINTER(wintypes.DWORD)]
user.GetWindowThreadProcessId.restype=wintypes.DWORD
user.GetMenuItemCount.argtypes=[wintypes.HMENU]
user.GetMenuItemID.argtypes=[wintypes.HMENU,ctypes.c_int]
user.GetMenuItemID.restype=wintypes.UINT
user.GetLayeredWindowAttributes.argtypes=[wintypes.HWND,ctypes.POINTER(wintypes.DWORD),ctypes.POINTER(ctypes.c_ubyte),ctypes.POINTER(wintypes.DWORD)]
gdi = ctypes.WinDLL('gdi32')
gdi.CreateCompatibleDC.argtypes = [wintypes.HDC]
gdi.CreateCompatibleDC.restype = wintypes.HDC
gdi.CreateCompatibleBitmap.argtypes = [wintypes.HDC,ctypes.c_int,ctypes.c_int]
gdi.CreateCompatibleBitmap.restype = wintypes.HBITMAP
gdi.SelectObject.argtypes = [wintypes.HDC,wintypes.HGDIOBJ]
gdi.SelectObject.restype = wintypes.HGDIOBJ
gdi.GetDIBits.argtypes = [wintypes.HDC,wintypes.HBITMAP,wintypes.UINT,wintypes.UINT,ctypes.c_void_p,ctypes.c_void_p,wintypes.UINT]
gdi.DeleteObject.argtypes = [wintypes.HGDIOBJ]
gdi.DeleteDC.argtypes = [wintypes.HDC]
gdi.BitBlt.argtypes=[wintypes.HDC,ctypes.c_int,ctypes.c_int,ctypes.c_int,ctypes.c_int,wintypes.HDC,ctypes.c_int,ctypes.c_int,wintypes.DWORD]
def window(name):
    end = time.monotonic()+8
    while time.monotonic()<end:
        handle = user.FindWindowW(name,None)
        if handle:return handle
        if child.poll() is not None:raise RuntimeError(f'Receiver exited {child.returncode}')
        time.sleep(.1)
    raise RuntimeError(f'Window missing: {name}')
def capture(handle,name,client=True,live=False):
    rect = wintypes.RECT()
    if client:
        assert user.GetClientRect(handle,ctypes.byref(rect))
        width,height = rect.right,rect.bottom
    else:
        assert user.GetWindowRect(handle,ctypes.byref(rect))
        width,height=rect.right-rect.left,rect.bottom-rect.top
    screen = user.GetDC(handle) if client else user.GetWindowDC(handle)
    dc = gdi.CreateCompatibleDC(screen)
    bitmap = gdi.CreateCompatibleBitmap(screen,width,height)
    old = gdi.SelectObject(dc,bitmap)
    try:
        if live:
            assert gdi.BitBlt(dc,0,0,width,height,screen,0,0,0x00CC0020), 'Live window capture failed'
        else:
            assert user.PrintWindow(handle,dc,3 if client else 2), 'PrintWindow failed'
        gdi.SelectObject(dc,old)
        header = ctypes.create_string_buffer(40)
        import struct
        struct.pack_into('<IiiHHIIiiII',header,0,40,width,-height,1,32,0,width*height*4,0,0,0,0)
        pixels = ctypes.create_string_buffer(width*height*4)
        assert gdi.GetDIBits(dc,bitmap,0,height,pixels,header,0) == height
        image = Image.frombytes('RGB',(width,height),pixels.raw,'raw','BGRX')
        image.save(QA/name)
        return image
    finally:
        gdi.DeleteObject(bitmap)
        gdi.DeleteDC(dc)
        user.ReleaseDC(handle,screen)
def bounds(handle):
    rect=wintypes.RECT()
    assert user.GetWindowRect(handle,ctypes.byref(rect))
    return rect.left,rect.top,rect.right-rect.left,rect.bottom-rect.top
def control_text(handle):
    buf=ctypes.create_unicode_buffer(512)
    user.SendMessageW(handle,0xD,512,ctypes.addressof(buf))
    return buf.value
profile = tempfile.TemporaryDirectory(prefix='airbridge-ui-')
env = os.environ.copy()
env['APPDATA'] = str(Path(profile.name)/'settings')
env['LOCALAPPDATA'] = str(Path(profile.name)/'local')
env['AIRBRIDGE_ROOT'] = str(Path(profile.name)/'runtime')
child = subprocess.Popen([str(ROOT/'AirBridge.exe'),'--receiver','--preview-fixture',str(Path(__file__).parent/'fixtures/test.h264')],env=env)
try:
    receiver = window('AirBridge.Rust.Receiver')
    user.SetWindowPos(receiver,None,80,80,1000,760,0x0040)
    time.sleep(2)
    capture(receiver,'qa-rust-receiver.png')
    assert not user.GetMenu(receiver), 'Toolbar replaces the legacy menu bar'
    shell=ctypes.WinDLL('shell32')
    shell.ExtractIconExW.argtypes=[wintypes.LPCWSTR,ctypes.c_int,ctypes.c_void_p,ctypes.c_void_p,wintypes.UINT]
    assert shell.ExtractIconExW(str(ROOT/'AirBridge.exe'),-1,None,None,0)==1, 'EXE must contain the actual app icon'
    user.PostMessageW(receiver,0x111,115,0)
    for _ in range(30):
        popup=user.FindWindowW('#32768',None)
        if popup:break
        time.sleep(.02)
    assert popup,'More menu did not open'
    menu=user.SendMessageW(popup,0x1E1,0,0)
    ids=[user.GetMenuItemID(menu,i) for i in range(user.GetMenuItemCount(menu))]
    assert ids==[102,110,103,104], f'More menu must not duplicate Settings: {ids}'
    user.PostMessageW(popup,0x100,27,0)
    time.sleep(.1)
    for selected in [112,113,111]:
        user.SendMessageW(receiver,0x111,selected,0)
        config = (Path(env['APPDATA'])/'AirPlay-Windows/settings.ini').read_text()
        assert f'mirror_smooth_scaling={int(selected == 112)}' in config
        assert f'mirror_unfiltered_scaling={int(selected == 113)}' in config
    user.PostMessageW(receiver,0x111,107,0)
    settings = window('AirBridge.Rust.Settings')
    title=ctypes.create_unicode_buffer(128)
    assert user.GetWindowTextW(settings,title,128) == 0, 'Redundant settings caption must be empty'
    assert not user.GetWindowLongPtrW(settings,-16) & 0xC00000, 'Settings should use its own title and close button'
    assert not user.GetDlgItem(receiver,109), 'Recording button must be removed'
    # System app theme is the default, without changing Windows personalization.
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER,r'Software\Microsoft\Windows\CurrentVersion\Themes\Personalize') as key:
            system_dark=winreg.QueryValueEx(key,'AppsUseLightTheme')[0] == 0
    except OSError:system_dark=False
    initial=capture(settings,'qa-system-theme.png')
    assert initial.getpixel((5,100)) == ((20,20,22) if system_dark else (245,245,247))
    for theme,expected,name in [(214,(245,245,247),'qa-settings-light.png'),(215,(20,20,22),'qa-settings-dark.png')]:
        user.SendMessageW(user.GetDlgItem(settings,theme),0xF5,0,0)
        time.sleep(.05)
        shot=capture(settings,name)
        assert shot.getpixel((5,100)) == expected, 'Theme should apply immediately'
    capture(settings,'qa-settings-dark-window.png',client=False)
    # Exercise the exact nonclient activation/paint sequence behind the inactive frame bug.
    user.SendMessageW(settings,0x86,1,0) # WM_NCACTIVATE
    user.SendMessageW(settings,0x85,1,0) # WM_NCPAINT
    active=capture(settings,'qa-frame-active.png',client=False,live=True)
    user.SendMessageW(settings,0x86,0,0)
    user.SendMessageW(settings,0x85,1,0)
    inactive=capture(settings,'qa-frame-inactive.png',client=False,live=True)
    from PIL import ImageChops
    assert ImageChops.difference(active,inactive).getbbox() is None, 'Losing focus must not draw a stock nonclient frame'
    user.SendMessageW(settings,0x86,1,0)
    user.SendMessageW(user.GetDlgItem(settings,214),0xF5,0,0)
    time.sleep(.25)
    user.SendMessageW(user.GetDlgItem(settings,215),0xF5,0,0)
    fade=user.FindWindowW('AirBridge.ThemeTransition',None)
    assert fade,'Theme change must have a single composited overlay'
    time.sleep(.06)
    alpha=ctypes.c_ubyte();flags=wintypes.DWORD();color=wintypes.DWORD()
    assert user.GetLayeredWindowAttributes(fade,ctypes.byref(color),ctypes.byref(alpha),ctypes.byref(flags))
    assert 0<alpha.value<255, f'Theme overlay must pass through intermediate opacity: {alpha.value}'
    time.sleep(.25)
    assert not user.FindWindowW('AirBridge.ThemeTransition',None),'Theme overlay must clean up after fading'
    user.SendMessageW(user.GetDlgItem(settings,214),0xF5,0,0)
    time.sleep(.25)
    rows=[line.split('\t') for line in (Path(__file__).parent/'locales/ui.tsv').read_text(encoding='utf-8').splitlines()]
    catalog={row[0]:row[1:] for row in rows[1:]}
    (QA/'locales').mkdir(exist_ok=True)
    for index,code in enumerate(rows[0][1:]):
        user.SendMessageW(settings,0x111,400+index,0)
        assert control_text(user.GetDlgItem(receiver,107))==catalog['settings'][index]
        config=(Path(env['APPDATA'])/'AirPlay-Windows/settings.ini').read_text()
        assert f'language={code}\n' in config,'Language choice must persist immediately'
        capture(settings,f'locales/{code}.png')
    user.SendMessageW(settings,0x111,401,0) # Russian baseline for the geometry screenshots.
    auto = user.GetDlgItem(settings,212)
    assert user.SendMessageW(auto,0xF0,0,0) == 1, 'Auto should be on by default'
    for id in [201,202]:assert not user.IsWindowEnabled(user.GetDlgItem(settings,id))
    capture(settings,'qa-rust-settings.png')
    user.SendMessageW(auto,0xF5,0,0) # BM_CLICK: opt out and restore manual fields
    for id in [201,202]:assert user.IsWindowEnabled(user.GetDlgItem(settings,id))
    def value(id):
        buf=ctypes.create_unicode_buffer(256)
        user.SendMessageW(user.GetDlgItem(settings,id),0xD,256,ctypes.addressof(buf))
        return buf.value
    def set_value(id,text):
        buffer=ctypes.create_unicode_buffer(text)
        user.SendMessageW(user.GetDlgItem(settings,id),0xC,0,ctypes.addressof(buffer))
    assert value(201)=='1920' and value(202)=='1080', 'Manual defaults should be Full HD'
    for id in [201,202,203]:
        user.SendMessageW(user.GetDlgItem(settings,id),0x102,ord('x'),0)
        assert 'x' not in value(id), 'Typed letters must be rejected'
        set_value(id,'abc')
        assert value(id)=='', f'Pasted/programmatic letters must be sanitized: id={id}, value={value(id)!r}'
        set_value(id,'99999')
        assert value(id)==('60' if id==203 else '8192'), 'Upper limit must apply immediately'
        set_value(id,'0')
        user.SendMessageW(settings,0x111,id | (0x200<<16),user.GetDlgItem(settings,id))
        assert value(id)==('1' if id==203 else '320'), 'Lower limit must apply on focus loss'
    for id,text in [(201,'1920'),(202,'1080'),(203,'60')]:set_value(id,text)
    time.sleep(.25)
    manual=capture(settings,'qa-ios-settings-manual.png')
    for label,box in [('width',(374,391,428,427)),('height',(450,391,504,427)),('FPS',(440,444,504,480))]:
        pixels=[]
        for y in range(box[1]+5,box[3]-5):
            for x in range(box[0]+2,box[2]-2):
                r,g,b=manual.getpixel((x,y))
                hit=(r<180 and g<190 and b<245) if label=='FPS' else (b>150 and r<80 and g>70)
                if hit:pixels.append(x)
        left=min(pixels)-box[0];right=box[2]-1-max(pixels)
        assert abs(left-right)<=2, f'{label} must be visually centered: {left=}, {right=}'
    for id in [200,201,202,203]:
        edit=user.GetDlgItem(settings,id)
        user.SendMessageW(edit,0x201,1,(8<<16)|5)
        user.SendMessageW(edit,0x202,0,(8<<16)|5)
        user.SendMessageW(edit,0xB1,999,999)
        info=GuiThreadInfo();info.cbSize=ctypes.sizeof(info)
        assert user.GetGUIThreadInfo(user.GetWindowThreadProcessId(edit,None),ctypes.byref(info))
        assert info.hwndCaret==edit, f'Caret must belong to the focused field {id}'
        assert info.rcCaret.bottom-info.rcCaret.top == (17 if id==200 else 15), (id,info.rcCaret.top,info.rcCaret.bottom)
        assert info.rcCaret.top == (1 if id==200 else 2), 'Caret should be inset within the text line'
    capture(settings,'qa-settings-light-window.png',client=False)
    # The thumb must pass through intermediate positions; track colors do not interpolate.
    user.SendMessageW(auto,0xF1,0,0)
    before=capture(auto,'qa-switch-off.png')
    user.SendMessageW(auto,0xF5,0,0)
    time.sleep(.045)
    middle=capture(auto,'qa-switch-middle.png')
    time.sleep(.25)
    after=capture(auto,'qa-switch-on.png')
    def thumb(image):
        xs=[x for x in range(4,49) if min(image.getpixel((x,17)))>240]
        return sum(xs)/len(xs)
    assert thumb(before)<thumb(middle)<thumb(after), 'Switch thumb must animate rather than jump'
    assert middle.getpixel((10,9)) == after.getpixel((10,9)), 'Track color should change immediately'
    before.save(QA/'qa-switch-animation.gif',save_all=True,append_images=[middle,after],duration=[60,100,400],loop=0)
    for id in [201,202]:assert not user.IsWindowEnabled(user.GetDlgItem(settings,id))
    user.SetWindowPos(settings,None,420,180,544,794,0x0040)
    user.SendMessageW(user.GetDlgItem(settings,216),0xF5,0,0)
    assert not user.IsWindowVisible(settings), 'Custom close button must dismiss settings'
    user.SendMessageW(user.GetDlgItem(receiver,107),0xF5,0,0)
    settings=window('AirBridge.Rust.Settings')
    assert bounds(settings)[:2]==(420,180), 'Settings position must survive closing and reopening'
    user.SendMessageW(user.GetDlgItem(settings,216),0xF5,0,0)
    capture(receiver,'qa-toolbar-after-click.png')
    user.PostMessageW(receiver,0x111,106,0)
    time.sleep(.3)
    capture(receiver,'qa-rust-rotation.png')
    user.PostMessageW(user.GetDlgItem(receiver,107),0x100,0x7a,0) # F11 with toolbar focus
    time.sleep(.3)
    assert not user.IsWindowVisible(user.GetDlgItem(receiver,107)), 'Toolbar must hide in fullscreen'
    capture(receiver,'qa-ios-fullscreen.png')
    user.PostMessageW(receiver,0x100,0x1b,0) # Escape restores the window
    time.sleep(.3)
    assert user.IsWindowVisible(user.GetDlgItem(receiver,107)), 'Toolbar must return after fullscreen'
    user.SetWindowPos(receiver,None,230,150,900,760,0x0040)
    user.PostMessageW(receiver,0x10,0,0)
    assert child.wait(timeout=10) == 0
    print('PASS: real receiver, GPU fixture, toolbar, scaling, settings switches, rotation, fullscreen and close')
    child = subprocess.Popen([str(ROOT/'AirBridge.exe'),'--receiver','--loopback','--no-mdns','--port','0'],env=env)
    receiver=window('AirBridge.Rust.Receiver')
    assert control_text(user.GetDlgItem(receiver,107))=='Настройки', 'Language choice must survive restart'
    assert bounds(receiver)==(230,150,900,760), f'Main placement must survive restart: {bounds(receiver)}'
    user.SetWindowPos(receiver,None,80,80,1000,760,0x0040)
    time.sleep(.5)
    capture(receiver,'qa-ios-landing.png')
    user.SetWindowPos(receiver,None,80,80,680,700,0x0040)
    time.sleep(.2)
    capture(receiver,'qa-ios-compact.png')
    # Exercise the real toolbar button, not only its WM_COMMAND handler.
    user.SendMessageW(user.GetDlgItem(receiver,107),0xF5,0,0)
    settings=window('AirBridge.Rust.Settings')
    assert bounds(settings)[:2]==(420,180), 'Settings placement must survive app restart'
    user.SendMessageW(user.GetDlgItem(settings,215),0xF5,0,0)
    time.sleep(.1)
    capture(receiver,'qa-landing-dark.png')
    user.SendMessageW(user.GetDlgItem(settings,213),0xF5,0,0)
    user.SendMessageW(user.GetDlgItem(settings,210),0xF5,0,0)
    assert user.SendMessageW(user.GetDlgItem(settings,210),0xF0,0,0) == 0
    user.SendMessageW(user.GetDlgItem(settings,108),0xF5,0,0)
    assert child.wait(timeout=10) == 2, 'Saving should request a receiver restart'
    config=(Path(env['APPDATA'])/'AirPlay-Windows/settings.ini').read_text()
    assert 'mirror_hwaccel=0' in config and 'mirror_auto_resolution=1' in config and 'theme_mode=system' in config
    assert 'max_fps=60' in config and 'mirror_width=1920' in config and 'mirror_height=1080' in config
    print('PASS: numeric-only input, min/max limits, Full HD manual defaults, switch animation, system/light/dark themes, caption cleanup and saved values')
    print('PASS: settings placement survives reopen/restart; main position and size survive restart')
    print('PASS: visual centering of dimension/FPS pills and native caret geometry in every edit field')
    print('PASS: 10 live languages and persistence, embedded app icon, cleaned More menu, composited theme crossfade and cleanup')
    print('PASS: active/inactive live window pixels match after nonclient activation and repaint')
finally:
    if child.poll() is None:
        child.terminate()
        child.wait(timeout=5)
    profile.cleanup()
