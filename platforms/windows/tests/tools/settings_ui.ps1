# 用独立合成设置运行真实 WinUI 宿主，检查全部页面并保存浅深色与窄窗口截图。
param(
    [Parameter(Mandatory = $true)][string]$Program,
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [switch]$InputOnly
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing, System.Windows.Forms
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class SettingsWindowProbe {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] public static extern bool SetDllDirectory(string path);
    [DllImport("msime_host_api.dll", CallingConvention = CallingConvention.Cdecl)] static extern IntPtr msime_client_default_preferences();
    [DllImport("msime_host_api.dll", CallingConvention = CallingConvention.Cdecl)] static extern void msime_client_string_free(IntPtr value);
    public static string DefaultPreferences() {
        IntPtr value = msime_client_default_preferences();
        try {
            int length = 0;
            while (Marshal.ReadByte(value, length) != 0) length++;
            byte[] bytes = new byte[length];
            Marshal.Copy(value, bytes, 0, length);
            return System.Text.Encoding.UTF8.GetString(bytes);
        } finally { msime_client_string_free(value); }
    }
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct MinMaxInfo {
        public Point Reserved, MaxSize, MaxPosition, MinTrackSize, MaxTrackSize;
    }
    [DllImport("user32.dll")] private static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr wparam, ref MinMaxInfo info);
    public static MinMaxInfo TrackLimits(IntPtr window) {
        var info = new MinMaxInfo();
        SendMessage(window, 0x0024, IntPtr.Zero, ref info);
        return info;
    }
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] static extern IntPtr GetWindowDpiAwarenessContext(IntPtr window);
    [DllImport("user32.dll")] static extern bool AreDpiAwarenessContextsEqual(IntPtr first, IntPtr second);
    public static bool PerMonitorV2(IntPtr window) {
        return AreDpiAwarenessContextsEqual(GetWindowDpiAwarenessContext(window), new IntPtr(-4));
    }
    [DllImport("user32.dll", EntryPoint = "SystemParametersInfoW", SetLastError = true)]
    static extern bool GetParameter(uint action, uint parameter, out uint value, uint flags);
    [DllImport("user32.dll", EntryPoint = "SystemParametersInfoW", SetLastError = true)]
    static extern bool SetParameter(uint action, uint parameter, IntPtr value, uint flags);
    public static uint WheelRouting() {
        uint value;
        if (!GetParameter(0x201C, 0, out value, 0)) throw new System.ComponentModel.Win32Exception();
        return value;
    }
    public static void WheelRouting(uint value) {
        if (!SetParameter(0x201D, 0, new IntPtr(value), 2)) throw new System.ComponentModel.Win32Exception();
        if (WheelRouting() != value) throw new InvalidOperationException("Wheel routing did not change");
    }
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [StructLayout(LayoutKind.Sequential)] private struct MouseInput {
        public int X, Y;
        public uint Data, Flags, Time;
        public IntPtr ExtraInfo;
    }
    [StructLayout(LayoutKind.Sequential)] private struct Input { public uint Type; public MouseInput Mouse; }
    [DllImport("user32.dll", SetLastError = true)] private static extern uint SendInput(uint count, Input[] inputs, int size);
    [DllImport("user32.dll")] private static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll")] private static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll")] private static extern IntPtr GetAncestor(IntPtr window, uint flags);
    [DllImport("user32.dll")] private static extern IntPtr WindowFromPoint(Point point);
    private static uint GuiThread(IntPtr window) {
        uint process;
        uint thread = GetWindowThreadProcessId(window, out process);
        if (thread == 0) throw new InvalidOperationException("Synthetic settings window has no GUI thread");
        return thread;
    }
    public static IntPtr MouseTarget(IntPtr owner, int x, int y) {
        var target = WindowFromPoint(new Point { X = x, Y = y });
        if (target == IntPtr.Zero || GetAncestor(target, 2) != owner)
            throw new InvalidOperationException("Slider point does not hit the synthetic settings window");
        return target;
    }
    public static string MouseThreads(IntPtr owner, IntPtr target) {
        return "root=" + GuiThread(owner) + ", target=" + GuiThread(target) + ", hwnd=" + target.ToInt64().ToString("X");
    }
    private static void SendMouse(MouseInput mouse) {
        var input = new Input { Mouse = mouse };
        if (SendInput(1, new[] { input }, Marshal.SizeOf(typeof(Input))) != 1)
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
    }
    public static bool LeftButtonDown() { return (GetAsyncKeyState(1) & 0x8000) != 0; }
    public static void MouseButton(bool pressed) {
        SendMouse(new MouseInput { Flags = pressed ? 0x0002u : 0x0004u });
    }
    public static void MoveMouse(int x, int y) {
        int left = GetSystemMetrics(76), top = GetSystemMetrics(77);
        int width = GetSystemMetrics(78), height = GetSystemMetrics(79);
        if (width <= 1 || height <= 1) throw new InvalidOperationException("Invalid virtual desktop bounds");
        SendMouse(new MouseInput {
            X = (int)Math.Round((x - left) * 65535.0 / (width - 1)),
            Y = (int)Math.Round((y - top) * 65535.0 / (height - 1)),
            Flags = 0x0001u | 0x4000u | 0x8000u
        });
    }
    [DllImport("user32.dll")] static extern void mouse_event(uint flags, uint x, uint y, uint data, UIntPtr extra);
    public static void Wheel(int delta) { mouse_event(0x0800, 0, 0, unchecked((uint)delta), UIntPtr.Zero); }
}
'@
. (Join-Path $PSScriptRoot 'settings_wheel.ps1')
. (Join-Path $PSScriptRoot 'settings_snapshot.ps1')
$programPath = (Resolve-Path -LiteralPath $Program).Path
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$outputPath = (Resolve-Path -LiteralPath $OutputDirectory).Path
$state = Join-Path ([IO.Path]::GetTempPath()) ('msime-settings-ui-' + [Guid]::NewGuid())
$oldState = $env:MSIME_CLIENT_STATE_DIR
$pages = @('typing', 'shortcuts', 'candidate', 'toolbar', 'about')
$process = $null
# 用仓库许可明确的合成皮肤验证真实设置卡片；颜色块让缺图和旧模式都能被屏幕像素发现。
# 屏幕比要的尺寸小时，SetWindowPos 撑不到要的大小，之后的「宽窗口」断言会在不相干的地方失败（Depot 的 Windows 机型主屏是 800×600，窗口停在最小尺寸 820×620）。设完尺寸先读回来，撑不到就直接说清楚是屏幕的问题。
function Assert-WindowSize([IntPtr]$handle, [int]$width, [int]$height) {
    $bounds = [System.Windows.Automation.AutomationElement]::FromHandle($handle).Current.BoundingRectangle
    if ($bounds.Width -lt $width -or $bounds.Height -lt $height) {
        $screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
        throw "Settings window is $([int]$bounds.Width)x$([int]$bounds.Height), not ${width}x${height}: the $($screen.Width)x$($screen.Height) screen is too small for this check"
    }
}

function Find-ById($parent, [string]$id) {
    $condition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::AutomationIdProperty, $id)
    return $parent.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
}
function Assert-NativeInput($window, [IntPtr]$handle) {
    if (-not [SettingsWindowProbe]::PerMonitorV2($handle)) { throw 'Settings window is not PerMonitorV2 DPI aware' }
    # 固定为最小窗口与内容较长的工具栏页，避免默认输入页的短标签页没有滚动范围。
    if (-not [SettingsWindowProbe]::SetWindowPos($handle, [IntPtr]::Zero, 0, 0, 800, 560, 0x0040)) { throw 'Wheel fixture resize failed' }
    Open-Page $window 'toolbar'
    $routing = [SettingsWindowProbe]::WheelRouting()
    $cursor = New-Object SettingsWindowProbe+Point
    if (-not [SettingsWindowProbe]::GetCursorPos([ref]$cursor)) { throw 'Could not save cursor position' }
    try {
        foreach ($mode in @(0, 2)) {
            # 同时覆盖关闭和开启「悬停时滚动非活动窗口」；不持久化系统设置，退出时恢复。
            [SettingsWindowProbe]::WheelRouting($mode)
            [SettingsWindowProbe]::SetForegroundWindow($handle) | Out-Null
            # 导航和窗口缩放都会异步提交布局；每次重取 UIA 节点，直到真实正文出现滚动范围。
            $scroll = $null
            $deadline = [DateTime]::UtcNow.AddSeconds(10)
            do {
                $content = Find-ById $window 'settings-content'
                if ($content) {
                    try {
                        [object]$pattern = $null
                        if ($content.TryGetCurrentPattern([System.Windows.Automation.ScrollPattern]::Pattern, [ref]$pattern) -and $pattern.Current.VerticallyScrollable) {
                            $scroll = $pattern
                            break
                        }
                    } catch [System.Windows.Automation.ElementNotAvailableException] {
                        # 页面重建时重新查询，不能把失效节点当成已完成布局。
                    }
                }
                Start-Sleep -Milliseconds 100
            } while ([DateTime]::UtcNow -lt $deadline)
            if (-not $scroll) {
                Save-Window $window "wheel-fixture-routing-$mode.png"
                throw 'Wheel fixture page does not overflow after layout'
            }
            $scroll.SetScrollPercent(-1, 0)
            Start-Sleep -Milliseconds 250
            $bounds = $content.Current.BoundingRectangle
            if (-not [SettingsWindowProbe]::SetCursorPos([int]$bounds.X + 10, [int]($bounds.Y + $bounds.Height / 2))) { throw 'Could not position wheel over the content margin' }
            if ([SettingsWindowProbe]::GetForegroundWindow() -ne $handle) { throw 'Settings window could not acquire foreground for wheel input' }
            $before = $scroll.Current.VerticalScrollPercent
            [SettingsWindowProbe]::Wheel(-360)
            $deadline = [DateTime]::UtcNow.AddSeconds(5)
            do {
                Start-Sleep -Milliseconds 100
                $after = $scroll.Current.VerticalScrollPercent
            } while ($after -le $before -and [DateTime]::UtcNow -lt $deadline)
            if ($after -le $before) { throw "Mouse wheel did not scroll the native page (routing=$mode)" }
            Write-Host "Native wheel routing=$mode moved from $before to $after"
        }
    } finally {
        [SettingsWindowProbe]::WheelRouting($routing)
        [SettingsWindowProbe]::SetCursorPos($cursor.X, $cursor.Y) | Out-Null
    }
}
function Find-ByName($parent, [string]$name) {
    $condition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, $name)
    return $parent.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
}
# 设置项的解释性说明收在标题旁的「?」里（main.cpp 的 info_tip），悬停或聚焦时才以 ToolTip 显示，平时不在界面上。按钮的读屏名称四端一致：有标题时是「<标题>的说明：<正文>」，没有标题时是「说明：<正文>」（main.cpp 的 info_tip_name）。
function Find-InfoTip($parent, [string]$title, [string]$text) {
    $name = if ($title) { "$($title)的说明：$text" } else { "说明：$text" }
    return Find-ByName $parent $name
}
function Assert-SelectedAccent($element) {
    if ($element.Current.IsOffscreen) { throw 'Selected control is outside the screenshot' }
    $bounds = $element.Current.BoundingRectangle
    # 只检查控件左侧的选中标记，侧栏或正文里的品牌绿不能替它通过。
    $bitmap = New-Object Drawing.Bitmap([int][Math]::Min(32, $bounds.Width), [int]$bounds.Height)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen([int]$bounds.X, [int]$bounds.Y, 0, 0, $bitmap.Size)
        $green = 0
        for ($x = 0; $x -lt $bitmap.Width; $x++) {
            for ($y = 0; $y -lt $bitmap.Height; $y++) {
                $color = $bitmap.GetPixel($x, $y)
                if ($color.G -gt 80 -and $color.G -gt $color.R * 1.3 -and $color.B -lt $color.G * 0.9) { $green++ }
            }
        }
        if ($green -lt 8) { throw 'Selected control still uses the system accent instead of brand green' }
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}
# 页首名称最多等多久。`scripts/test-windows-settings-page-readiness.py` 只加载下面两个函数来测它们，把它调短：那里有一项要等满超时，按 15 秒等，加上 pwsh 冷启动和 Add-Type，在并行跑检查的 CI 机上会超过外层的 40 秒。
$script:pageIntroNameSeconds = 15
function Wait-PageIntroName($window, [string]$page) {
    # 节点先出现、名称稍后才可读；每轮重新查询，返回当次名称，避免断言再读到重建中的节点。
    $deadline = [DateTime]::UtcNow.AddSeconds($script:pageIntroNameSeconds)
    do {
        try {
            $content = Find-ById $window 'settings-content'
            if ($content) {
                if (Find-ById $content 'settings-load-error') { throw 'Synthetic preferences were rejected by the real host' }
                $intro = Find-ById $content "settings-page-$page"
                if ($intro) {
                    $name = $intro.Current.Name
                    if (-not [string]::IsNullOrWhiteSpace($name)) { return $name }
                }
            }
        } catch [System.Windows.Automation.ElementNotAvailableException] {
            # 页面异步重建会让刚查到的节点失效，下一轮重新查询。
        }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Page did not expose a name: $page"
}
function Open-Page($window, [string]$page) {
    # 下拉框等弹出层刚关闭时，UIA 树要稍晚才把侧栏交回来；等待导航项而非固定延时。
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $item = Find-ById $window $page
        if ($item) { break }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    if (-not $item) { throw "Missing native page: $page" }
    $item.GetCurrentPattern([System.Windows.Automation.ScrollItemPattern]::Pattern).ScrollIntoView()
    # 滚动会重建虚拟化的导航节点；重新查到且可选中才继续，不能立即解引用暂时消失的节点。
    $selected = $false
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $item = Find-ById $window $page
        if ($item) {
            try {
                [object]$selection = $null
                if ($item.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$selection)) {
                    $selection.Select()
                    $selected = $true
                }
            } catch [System.Windows.Automation.ElementNotAvailableException] {
                # 查找与选择之间节点也可能失效，下一次轮询重新取得。
            }
        }
        if (-not $selected) { Start-Sleep -Milliseconds 200 }
    } while (-not $selected -and [DateTime]::UtcNow -lt $deadline)
    if (-not $selected) { throw "Native page could not be selected after scrolling: $page" }
    Wait-PageIntroName $window $page | Out-Null
}
function Assert-PinyinAutocorrect($window, [string]$scheme) {
    # 「高级选项」的全拼纠错 / 双拼纠错组（#10210）照共享页：只在全拼、双拼方案下出现，标题和说明按方案写，两个开关都写 quanpin 下的字段、默认开。全拼、双拼方案下都经实际开关各关一次再打开，每次只改自己的字段，quanpin 下其他字段保持原值；关着时整页重画一次，开关要读回关；其他方案下这一组不画。
    $titles = @{
        quanpin = @('全拼纠错', '全拼邻键纠错', '全拼乱序纠错')
        shuangpin = @('双拼纠错', '双拼邻键纠错', '双拼乱序纠错')
    }
    $descriptions = @{
        quanpin = '把前后颠倒的两个字母纠回来，例如 hsang → shang、gau → gua。关闭后不再按乱序猜测拼写。两个开关都关闭时，漏打、多打字母的纠错（shng → shang）也一起关闭。'
        shuangpin = '把两键编码前后颠倒的音节纠回来，同样只用于三个音节以上的整句。关闭后不再按乱序猜测编码。'
    }
    Select-PageTab $window 'input-tab-advanced' '高级选项'
    $content = Find-ById $window 'settings-content'
    foreach ($family in @('quanpin', 'shuangpin')) {
        foreach ($name in $titles[$family]) {
            $shown = [bool](Find-ByName $content $name)
            if ($shown -ne ($family -eq $scheme)) { throw "Autocorrect row $name shown=$shown under scheme $scheme" }
        }
    }
    if ($titles.ContainsKey($scheme) -and -not (Find-InfoTip $content $titles[$scheme][2] $descriptions[$scheme])) { throw "Autocorrect transposition description does not follow the shared page under $scheme" }
    if ($titles.ContainsKey($scheme)) {
        $path = Join-Path $state 'preferences.json'
        $toggleable = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::IsTogglePatternAvailableProperty, $true)
        foreach ($row in @(@($titles[$scheme][1], 'autocorrect_neighbor'), @($titles[$scheme][2], 'autocorrect_transposition'))) {
            $name = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, $row[0])
            $condition = New-Object System.Windows.Automation.AndCondition($name, $toggleable)
            $toggle = (Find-ById $window 'settings-content').FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
            if (-not $toggle -or -not $toggle.Current.IsEnabled) { throw "Advanced tab is missing the $($row[0]) switch" }
            $pattern = $toggle.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern)
            if ($pattern.Current.ToggleState -ne [System.Windows.Automation.ToggleState]::On) { throw "$($row[0]) must default to on" }
            $before = ((Read-SettingsSnapshotText $path) | ConvertFrom-Json).preferences.quanpin
            if ($before.($row[1]) -ne $true) { throw "Synthetic preferences do not start with quanpin.$($row[1]) on" }
            foreach ($expected in @($false, $true)) {
                $pattern.Toggle()
                $deadline = [DateTime]::UtcNow.AddSeconds(10)
                do {
                    Start-Sleep -Milliseconds 200
                    $quanpin = ((Read-SettingsSnapshotText $path) | ConvertFrom-Json).preferences.quanpin
                } while ($quanpin.($row[1]) -ne $expected -and [DateTime]::UtcNow -lt $deadline)
                if ($quanpin.($row[1]) -ne $expected) { throw "$($row[0]) was not saved as quanpin.$($row[1]) = $expected" }
                foreach ($field in $before.PSObject.Properties) {
                    if ($field.Name -ne $row[1] -and $quanpin.($field.Name) -ne $field.Value) { throw "Saving $($row[0]) changed quanpin.$($field.Name)" }
                }
                if (-not $expected) {
                    # 存着的值与回退值（开）不同时整页重画，开关必须读回关，证明它读的是 quanpin 下这个字段而不是只显示回退值。
                    Select-PageTab $window 'input-tab-scheme' '输入方案'
                    Select-PageTab $window 'input-tab-advanced' '高级选项'
                    $toggle = (Find-ById $window 'settings-content').FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
                    if (-not $toggle) { throw "Advanced tab lost the $($row[0]) switch after a redraw" }
                    $pattern = $toggle.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern)
                    if ($pattern.Current.ToggleState -ne [System.Windows.Automation.ToggleState]::Off) { throw "$($row[0]) does not read back quanpin.$($row[1]) = false after a redraw" }
                }
            }
        }
        Write-Host "$scheme autocorrect switches default on, read back and save their own quanpin fields and keep the rest"
    }
    Select-PageTab $window 'input-tab-scheme' '输入方案'
}
function Find-CandidatePageSizeSlider($window) {
    $nameCondition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, '每页候选项数量')
    $typeCondition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Slider)
    $condition = New-Object System.Windows.Automation.AndCondition($nameCondition, $typeCondition)
    return $window.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
}
function Show-CandidatePageSizeSlider($window) {
    $content = Find-ById $window 'settings-content'
    $scroll = $content.GetCurrentPattern([System.Windows.Automation.ScrollPattern]::Pattern)
    $seekFromTop = $false
    $lastBounds = $null
    $lastPercent = -1.0
    $stable = 0
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $slider = Find-CandidatePageSizeSlider $window
        if (-not $slider) { throw '候选数量滑块缺失' }
        $bounds = $slider.Current.BoundingRectangle
        $viewport = $content.Current.BoundingRectangle
        # 留出预览高度变化的空间，避免拖动后这一行再次被挤出视口。
        $visible = -not $slider.Current.IsOffscreen -and $bounds.Height -gt 0 -and
            $bounds.Top -ge $viewport.Top -and $bounds.Bottom -le $viewport.Bottom
        if ($visible -and (-not $scroll.Current.VerticallyScrollable -or
            $bounds.Bottom -le $viewport.Top + $viewport.Height / 2 -or
            $scroll.Current.VerticalScrollPercent -ge 100)) {
            # Scroll 返回时滚动动画仍可能在移动；坐标与偏移连续稳定后才能按下滑块柄。
            $percent = $scroll.Current.VerticalScrollPercent
            if ($null -ne $lastBounds -and $bounds.Equals($lastBounds) -and
                [Math]::Abs($percent - $lastPercent) -lt 0.001) { $stable++ } else { $stable = 0 }
            if ($stable -ge 3) { return $slider }
            $lastBounds = $bounds
            $lastPercent = $percent
        } else {
            $stable = 0
            $lastBounds = $null
            if (-not $scroll.Current.VerticallyScrollable) { throw '候选数量滑块不可见且正文无法滚动' }
            # 离屏元素可能没有可用矩形，从顶部向下找，不能由空矩形推断滚动方向。
            if (-not $seekFromTop) {
                $scroll.SetScrollPercent(-1, 0)
                $seekFromTop = $true
            } else {
                $scroll.Scroll([System.Windows.Automation.ScrollAmount]::NoAmount, [System.Windows.Automation.ScrollAmount]::SmallIncrement)
            }
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw '候选数量滑块未滚动到可视区域'
}
function Find-SliderValue($window, $slider) {
    $bounds = $slider.Current.BoundingRectangle
    $condition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Text)
    $labels = $window.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition)
    $found = $null
    foreach ($label in $labels) {
        if ($label.Current.IsOffscreen -or $label.Current.Name -notmatch '^\d+$') { continue }
        $rect = $label.Current.BoundingRectangle
        if ($rect.Left -ge $bounds.Right -and $rect.Left -lt $bounds.Right + 100 -and
            $rect.Top -lt $bounds.Bottom -and $rect.Bottom -gt $bounds.Top) {
            if ($found) { throw '滑块右侧存在多个数值标签，无法唯一定位' }
            $found = $label
        }
    }
    return $found
}
function Assert-CandidatePageSizeLabel($window, $slider, [int]$expected) {
    $range = $slider.GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern)
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    while ($range.Current.Value -ne $expected -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 100 }
    if ($range.Current.Value -ne $expected) { throw "候选数量滑块未变为 $expected，实际为 $($range.Current.Value)" }
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        $bounds = $slider.Current.BoundingRectangle
        $viewport = (Find-ById $window 'settings-content').Current.BoundingRectangle
        if ($slider.Current.IsOffscreen -or $bounds.Height -le 0 -or
            $bounds.Top -lt $viewport.Top -or $bounds.Bottom -gt $viewport.Bottom) { throw '数值变化后候选数量滑块离开可视区域' }
        $label = Find-SliderValue $window $slider
        if ($label -and -not $label.Current.IsOffscreen -and $label.Current.Name -eq [string]$expected) { break }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    if (-not $label -or $label.Current.IsOffscreen -or $label.Current.Name -ne [string]$expected) { throw "候选数量右侧标签未更新为 $expected" }
    if ($slider.GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern).Current.Value -ne $expected) { throw '候选数量标签与滑块数值不一致' }
}
function Wait-CandidatePageSizeSaved([int]$expected) {
    $path = Join-Path $state 'preferences.json'
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    $saved = $null
    do {
        $saved = (Read-SettingsSnapshotText $path) | ConvertFrom-Json
        if ($saved.preferences.candidate_page_size -eq $expected) { return }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "候选数量未保存：期望 $expected，文件里是 $($saved.preferences.candidate_page_size)，revision=$($saved.revision)"
}
function Find-SliderThumb($slider) {
    # WinUI 默认模板把滑块柄放在 Raw view，只遍历当前滑块，不能扫描右侧标签。
    $walker = [System.Windows.Automation.TreeWalker]::RawViewWalker
    $pending = New-Object 'System.Collections.Generic.Stack[System.Windows.Automation.AutomationElement]'
    $pending.Push($slider)
    $found = $null
    while ($pending.Count -gt 0) {
        $node = $pending.Pop()
        if ($node.Current.ControlType -eq [System.Windows.Automation.ControlType]::Thumb -and -not $node.Current.IsOffscreen) {
            if ($found) { throw '滑块内存在多个可见滑块柄' }
            $found = $node
        }
        $child = $walker.GetFirstChild($node)
        while ($child) {
            $pending.Push($child)
            $child = $walker.GetNextSibling($child)
        }
    }
    return $found
}
function Assert-CandidatePageSizeDrag($window, [string]$screenshotPrefix) {
    $handle = [IntPtr]$window.Current.NativeWindowHandle
    [SettingsWindowProbe]::SetForegroundWindow($handle) | Out-Null
    if ([SettingsWindowProbe]::GetForegroundWindow() -ne $handle) { throw '拖动检查要求合成设置窗口在前台' }
    $cursor = New-Object SettingsWindowProbe+Point
    if (-not [SettingsWindowProbe]::GetCursorPos([ref]$cursor)) { throw '拖动检查需要交互桌面' }
    try {
        foreach ($expected in @(9, 3)) {
            $slider = Show-CandidatePageSizeSlider $window
            if (-not $slider -or $slider.Current.IsOffscreen -or -not $slider.Current.IsEnabled) { throw '候选数量滑块不可拖动' }
            $thumb = Find-SliderThumb $slider
            if (-not $thumb -or $thumb.Current.IsOffscreen) { throw '候选数量滑块缺少可见滑块柄' }
            $bounds = $slider.Current.BoundingRectangle
            $grip = $thumb.Current.BoundingRectangle
            $startX = [int]($grip.Left + $grip.Width / 2)
            $y = [int]($grip.Top + $grip.Height / 2)
            $endX = if ($expected -eq 9) { [int]($bounds.Right - 1) } else { [int]($bounds.Left + 1) }
            Write-Host "候选数量拖动起点=($startX,$y)，终点=($endX,$y)，滑块=$bounds，滑块柄=$grip"
            if (-not [SettingsWindowProbe]::SetCursorPos($startX, $y)) { throw '无法将鼠标放到滑块柄上' }
            $inputTarget = [SettingsWindowProbe]::MouseTarget($handle, $startX, $y)
            Write-Host "候选数量鼠标命中线程：$([SettingsWindowProbe]::MouseThreads($handle, $inputTarget))"
            try {
                if ([SettingsWindowProbe]::LeftButtonDown()) { throw '拖动前鼠标左键已按下' }
                [SettingsWindowProbe]::MouseButton($true)
                # WinUI 的指针捕获不保证出现在 HWND 捕获字段；按下与移动统一进 SendInput 队列，保持输入顺序。
                $deadline = [DateTime]::UtcNow.AddSeconds(5)
                while (-not [SettingsWindowProbe]::LeftButtonDown() -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 50 }
                if (-not [SettingsWindowProbe]::LeftButtonDown()) { throw '鼠标左键按下未进入系统输入状态' }
                # 分步输入形成真实拖动；读取标签前先完成第一次数值变化。
                for ($step = 1; $step -le 8; $step++) {
                    $x = [int]($startX + ($endX - $startX) * $step / 8)
                    [SettingsWindowProbe]::MoveMouse($x, $y)
                    Start-Sleep -Milliseconds 25
                }
                if (-not [SettingsWindowProbe]::LeftButtonDown()) { throw '读取拖动数值前鼠标左键已松开' }
                Assert-CandidatePageSizeLabel $window $slider $expected
                if ($expected -eq 9) { Save-Window $window "$screenshotPrefix-candidate-page-size-drag-held-9.png" }
            } finally {
                [SettingsWindowProbe]::MouseButton($false)
            }
            Wait-CandidatePageSizeSaved $expected
            Assert-CandidatePageSizeLabel $window $slider $expected
        }
    } finally {
        [SettingsWindowProbe]::SetCursorPos($cursor.X, $cursor.Y) | Out-Null
    }
    Write-Host '候选数量真实鼠标拖动：按住、松开及保存验证通过'
}
function Assert-CandidatePageSize($window, [string]$screenshotPrefix) {
    foreach ($expected in @(3, 9, 4, 6)) {
        $slider = Show-CandidatePageSizeSlider $window
        if (-not $slider -or -not $slider.Current.IsEnabled) { throw '候选数量滑块不可用' }
        $savedBefore = (Read-SettingsSnapshotText (Join-Path $state 'preferences.json')) | ConvertFrom-Json
        Write-Host "候选数量 $screenshotPrefix 操作前：滑块=$($slider.GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern).Current.Value)，文件=$($savedBefore.preferences.candidate_page_size)，revision=$($savedBefore.revision)，目标=$expected"
        $slider.SetFocus()
        $range = $slider.GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern)
        $range.SetValue($expected)
        # 检查同一个可见数值标签，不能用滑块 tooltip 或保存成功替代界面更新。
        Assert-CandidatePageSizeLabel $window $slider $expected
        Wait-CandidatePageSizeSaved $expected
        if ($expected -eq 9) { Save-Window $window "$screenshotPrefix-candidate-page-size-9.png" }
    }
    Write-Host '候选数量 UIA 数值变更、可见标签与保存验证通过，开始真实鼠标拖动'
    Assert-CandidatePageSizeDrag $window $screenshotPrefix
    # 拖动最后停在 3；恢复成 6 后再检查切页，保持后续合成页面的基线一致。
    $slider = Show-CandidatePageSizeSlider $window
    $slider.GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern).SetValue(6)
    Assert-CandidatePageSizeLabel $window $slider 6
    Wait-CandidatePageSizeSaved 6
    Open-Page $window 'toolbar'
    Open-Page $window 'candidate'
    Select-PageTab $window 'candidate-tab-layout' '候选项排列方式'
    $slider = Show-CandidatePageSizeSlider $window
    if (-not $slider) { throw '切页后候选数量滑块缺失' }
    $slider.SetFocus()
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        $label = Find-SliderValue $window $slider
        if ($label) { break }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    if (-not $slider -or -not $label -or $label.Current.Name -ne '6' -or
        $slider.GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern).Current.Value -ne 6) { throw '切页后候选数量未恢复已保存值' }
    Write-Host '候选数量滑块、右侧数值、保存及切页恢复验证通过'
}
function Select-ComboChoice($window, [string]$id, [string]$label) {
    $combo = Find-ById $window $id
    if (-not $combo) { throw "Missing combo: $id" }
    $combo.SetFocus()
    $combo.GetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        Start-Sleep -Milliseconds 200
        $item = Find-ByName $combo $label
    } while (-not $item -and [DateTime]::UtcNow -lt $deadline)
    if (-not $item) { throw "Missing combo option: $label" }
    $item.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Select()
    $combo.GetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern).Collapse()
}
# 仅操作独立合成窗口，验证录键经过原生事件、共享预检及真实偏好保存。
function Start-SettingsWindow([string]$route) {
    # 进程先记进脚本作用域的 $process，窗口没打开时 finally 也能把它收掉。
    $script:process = Start-Process -FilePath $programPath -ArgumentList "--route=$route" -PassThru
    $deadline = [DateTime]::UtcNow.AddSeconds(60)
    do {
        $script:process.Refresh()
        if ($script:process.HasExited) { throw "Settings exited before opening: $($script:process.ExitCode)" }
        if ($script:process.MainWindowHandle -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($script:process.MainWindowHandle -eq [IntPtr]::Zero) { throw 'Settings window did not open in 60 seconds' }
    return [System.Windows.Automation.AutomationElement]::FromHandle($script:process.MainWindowHandle)
}
function Select-PageTab($window, [string]$id, [string]$expected) {
    # 页内标签照 macOS 的 useSettingsTabs：按钮 ID 是 <前缀>-<标签 id>，选中的那个 ItemStatus 是「已选择」；切换后整页重画，等这块内容里的一个名字出现，而不是固定延时。
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $tab = Find-ById $window $id
        if ($tab -and $tab.Current.IsEnabled) { break }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    if (-not $tab) { throw "Missing page tab: $id" }
    if ($tab.Current.ItemStatus -ne '已选择') { $tab.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke() }
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        Start-Sleep -Milliseconds 200
        $tab = Find-ById $window $id
        $content = Find-ById $window 'settings-content'
        $shown = $content -and (Find-ByName $content $expected)
    } while ((-not $tab -or $tab.Current.ItemStatus -ne '已选择' -or -not $shown) -and [DateTime]::UtcNow -lt $deadline)
    if (-not $tab -or $tab.Current.ItemStatus -ne '已选择' -or -not $shown) { throw "Page tab $id did not show $expected" }
}
function Save-Window($element, [string]$file) {
    $bounds = $element.Current.BoundingRectangle
    if ($bounds.Width -lt 600 -or $bounds.Height -lt 400) { throw 'Settings window bounds are invalid' }
    $bitmap = New-Object Drawing.Bitmap([int]$bounds.Width, [int]$bounds.Height)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen([int]$bounds.X, [int]$bounds.Y, 0, 0, $bitmap.Size)
        $colors = New-Object 'System.Collections.Generic.HashSet[int]'
        for ($x = 10; $x -lt $bitmap.Width; $x += 37) {
            for ($y = 10; $y -lt $bitmap.Height; $y += 29) {
                $colors.Add($bitmap.GetPixel($x, $y).ToArgb()) | Out-Null
            }
        }
        if ($colors.Count -lt 8) { throw 'Screenshot is blank; the runner needs an interactive desktop' }
        $bitmap.Save((Join-Path $outputPath $file), [Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}
try {
    New-Item -ItemType Directory -Force $state | Out-Null
    $env:MSIME_CLIENT_STATE_DIR = $state
    if (-not [SettingsWindowProbe]::SetDllDirectory([IO.Path]::GetDirectoryName($programPath))) { throw 'Settings DLL path could not be set' }
    $defaults = [SettingsWindowProbe]::DefaultPreferences() | ConvertFrom-Json
    if (-not $defaults.ok) { throw 'Default preferences could not be loaded' }
    $themes = if ($InputOnly) { @('light') } else { @('light', 'dark') }
    foreach ($theme in $themes) {
        $preferences = $defaults.value
        $preferences.theme = $theme
        $preferences.scheme = 'quanpin'
        $preferences.default_ime_mode = 'chinese'
        $document = @{format_version = 1; revision = 1; preferences = $preferences}
        [IO.File]::WriteAllText((Join-Path $state 'preferences.json'), ($document | ConvertTo-Json -Depth 64))
        $window = Start-SettingsWindow 'settings:input'
        # 检查窗口真正交给系统的拖动尺寸边界，不能只检查 XAML 内部的最小宽度。
        $limits = [SettingsWindowProbe]::TrackLimits($process.MainWindowHandle)
        if ($limits.MinTrackSize.X -lt 800 -or $limits.MinTrackSize.Y -lt 560) { throw 'Native window minimum size was not applied' }
        # 窗口标题与 macOS 设置窗口相同，写作「<产品名> · 设置」。
        if (-not $window.Current.Name.EndsWith(' · 设置')) { throw "Unexpected settings window title: $($window.Current.Name)" }
        Assert-NativeInput $window $process.MainWindowHandle
        if ($InputOnly) {
            $process.CloseMainWindow() | Out-Null
            if (-not $process.WaitForExit(10000) -or $process.ExitCode -ne 0) { throw 'Settings input probe failed to close cleanly' }
            $process = $null
            continue
        }
        foreach ($width in @(1000, 800)) {
            if (-not [SettingsWindowProbe]::SetWindowPos($process.MainWindowHandle, [IntPtr]::Zero, 0, 0, $width, 780, 0x0040)) { throw 'Window resize failed' }
            Assert-WindowSize $process.MainWindowHandle $width 780
            foreach ($page in $pages) {
                if ($process.HasExited) { throw "Settings exited on $page" }
                # 复用导航等待，既验证正文身份，也处理侧栏虚拟化。
                Open-Page $window $page
                $content = Find-ById $window 'settings-content'
                if ($content.Current.BoundingRectangle.Width -lt 480) { throw "No usable content column on $page" }
                # 与 macOS 一样，标题栏里只有页名，没有跨页面的历史返回按钮。
                if (Find-ByName $window '返回') { throw "Settings title bar still has a history back button on $page" }
                # 页首照 macOS 的水杉娘引导：除「关于」外，正文顶部是一句说明配插画，说明带 settings-page-<页面>；「关于」没有引导，这个 ID 落在品牌卡的产品名上。
                $introName = Wait-PageIntroName $window $page
                $content = Find-ById $window 'settings-content'
                if ($page -eq 'about') {
                    if (-not $window.Current.Name.StartsWith($introName + ' · ')) { throw "About page identifies itself as '$introName' instead of the product name" }
                } elseif (-not $introName.EndsWith('。')) {
                    throw "Page intro of $page is not a sentence: '$introName'"
                }
                # 再等一轮呈现，避免 UIA 树已经更新而屏幕仍是上一页的画面。
                Start-Sleep -Milliseconds 500
                if ($page -eq 'typing') {
                    Select-PageTab $window 'input-tab-scheme' '输入方案'
                    if (-not (Find-ByName (Find-ById $window 'settings-content') '中文方案')) { throw 'Typing page is missing the scheme group' }
                }
                if ($page -eq 'shortcuts') {
                    Select-PageTab $window 'shortcuts-tab-settings' '按键设置'
                    Assert-SettingsWheelHitRegions $window $process.MainWindowHandle "$theme-$width"
                }
                if ($page -eq 'candidate') {
                    Select-PageTab $window 'candidate-tab-layout' '布局'
                    Assert-CandidatePageSize $window "$theme-$width"
                    $content = Find-ById $window 'settings-content'
                    foreach ($name in @('每页候选项数量', '跟随光标', '在游戏中显示候选窗口')) {
                        if (-not (Find-ByName $content $name)) { throw "Candidate layout tab is missing: $name" }
                    }
                }
                if ($page -eq 'toolbar') {
                    if (-not (Find-ByName $content '工具栏大小')) { throw 'Toolbar size select is missing' }
                }
                if ($page -eq 'about') {
                    Select-PageTab $window 'about-tab-version' '版本'
                    $content = Find-ById $window 'settings-content'
                    if (-not (Find-ByName $content '当前版本')) { throw 'About version tab is missing 当前版本' }
                }
                Save-Window $window "$theme-$width-$page.png"
                Write-Host "$theme $width $page rendered"
            }
        }
        # 通过真实单选控件切换方案，再读合成偏好确认保存成功，避免只验证静态页面。方案卡片在「输入方案」标签里。
        Open-Page $window 'typing'
        Select-PageTab $window 'input-tab-scheme' '输入方案'
        Start-Sleep -Milliseconds 700
        foreach ($scheme in @('shuangpin', 'quanpin')) {
            $radio = Find-ById $window "scheme-$scheme"
            if (-not $radio -or -not $radio.Current.IsEnabled) { throw "Scheme option is unavailable: $scheme" }
            $radio.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Select()
            $deadline = [DateTime]::UtcNow.AddSeconds(10)
            do {
                Start-Sleep -Milliseconds 200
                $saved = (Read-SettingsSnapshotText (Join-Path $state 'preferences.json')) | ConvertFrom-Json
            } while ($saved.preferences.scheme -ne $scheme -and [DateTime]::UtcNow -lt $deadline)
            if ($saved.preferences.scheme -ne $scheme) { throw "Scheme was not saved: $scheme" }
            Assert-SelectedAccent $radio
            if ($scheme -eq 'shuangpin') {
                Open-Page $window 'toolbar'
                $deadline = [DateTime]::UtcNow.AddSeconds(10)
                do {
                    Start-Sleep -Milliseconds 200
                    $badge = Find-ByName (Find-ById $window 'settings-content') '双'
                } while (-not $badge -and [DateTime]::UtcNow -lt $deadline)
                if (-not $badge -or $badge.Current.IsOffscreen) { throw 'Toolbar preview does not reflect scheme: shuangpin' }
                Open-Page $window 'typing'
                Select-PageTab $window 'input-tab-scheme' '输入方案'
            }
            Assert-PinyinAutocorrect $window $scheme
        }
        # 侧栏搜索照 macOS：有查询时页面列表整个换成结果列表，选中结果会切到它所在的页与标签；Esc 清空查询，页面列表回来。
        $search = Find-ById $window 'settings-search'
        $condition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.ControlTypeProperty, [System.Windows.Automation.ControlType]::Edit)
        $input = $search.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
        if (-not $input -and $search.Current.ControlType -eq [System.Windows.Automation.ControlType]::Edit) { $input = $search }
        if (-not $input) { throw 'Settings search input is missing' }
        $input.SetFocus()
        $input.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue('每页候选项数量')
        $deadline = [DateTime]::UtcNow.AddSeconds(10)
        do {
            Start-Sleep -Milliseconds 200
            $results = Find-ById $window 'settings-search-results'
            $first = if ($results) { Find-ById $results 'settings-search-result-0' } else { $null }
        } while (-not $first -and [DateTime]::UtcNow -lt $deadline)
        if (-not $first -or $first.Current.Name -ne '每页候选项数量') { throw 'Search results do not start with the matching row' }
        if (-not $first.Current.HelpText) { throw 'Search result does not name its page and tab' }
        Save-Window $window "$theme-search-results.png"
        $input.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue('没有这个设置项')
        $deadline = [DateTime]::UtcNow.AddSeconds(10)
        do {
            Start-Sleep -Milliseconds 200
            $results = Find-ById $window 'settings-search-results'
            $empty = if ($results) { Find-ByName $results '没有匹配的设置' } else { $null }
        } while (-not $empty -and [DateTime]::UtcNow -lt $deadline)
        if (-not $empty) { throw 'Search without matches does not say 没有匹配的设置' }
        $input.SetFocus()
        [System.Windows.Forms.SendKeys]::SendWait('{ESC}')
        $deadline = [DateTime]::UtcNow.AddSeconds(10)
        do {
            Start-Sleep -Milliseconds 200
            $item = Find-ById $window 'typing'
        } while ((-not $item -or $item.Current.IsOffscreen) -and [DateTime]::UtcNow -lt $deadline)
        $results = Find-ById $window 'settings-search-results'
        if (-not $item -or $item.Current.IsOffscreen -or ($results -and -not $results.Current.IsOffscreen)) { throw 'Escape did not clear the search and bring the page list back' }
        $process.CloseMainWindow() | Out-Null
        if (-not $process.WaitForExit(10000)) { throw 'Settings window failed to close' }
        if ($process.ExitCode -ne 0) { throw "Settings exited with $($process.ExitCode)" }
        $process = $null
    }
} catch {
    # 失败时保留合成窗口，便于区分产品布局问题与 UIA 时序问题。
    if ($process -and -not $process.HasExited) {
        try {
            Save-Window ([System.Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)) 'failure.png'
        } catch { Write-Warning 'Could not capture the failing synthetic window' }
    }
    throw
} finally {
    if ($process -and -not $process.HasExited) { Stop-Process -Id $process.Id -Force }
    [SettingsWindowProbe]::SetDllDirectory($null) | Out-Null
    $env:MSIME_CLIENT_STATE_DIR = $oldState
    Remove-Item -LiteralPath $state -Recurse -Force -ErrorAction SilentlyContinue
}

exit 0
