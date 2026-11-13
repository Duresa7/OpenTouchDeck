<#
.SYNOPSIS
Prints the AppUserModelID the Windows shell resolves for a window, as JSON.

.DESCRIPTION
A process that calls SetCurrentProcessExplicitAppUserModelID has no documented way to be read
from outside. The taskbar asks the shell's application resolver, an undocumented COM object, to
group windows; this asks it the same question. tray.ps1 runs it in a process of its own, so a
resolver that changed shape in a later Windows fails this one check and nothing else.

.EXAMPLE
powershell -File .github/scripts/windows-evidence/aumid.ps1 -Hwnd 1312345
#>
[CmdletBinding()]
param([Parameter(Mandatory)][long]$Hwnd)
$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace TouchDeckEvidence {
    // IApplicationResolver as Windows 8 and later lay it out.
    [ComImport, Guid("DE25675A-72DE-44B4-9373-05170450C140"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IApplicationResolver {
        [PreserveSig] int GetAppIDForShortcut(IntPtr item, out IntPtr appId);
        [PreserveSig] int GetAppIDForShortcutObject(IntPtr link, IntPtr item, out IntPtr appId);
        [PreserveSig] int GetAppIDForWindow(IntPtr hwnd, [MarshalAs(UnmanagedType.LPWStr)] out string appId,
            out int pinningPrevented, out int explicitAppId, out int embeddedShortcutValid);
        [PreserveSig] int GetAppIDForProcess(uint pid, [MarshalAs(UnmanagedType.LPWStr)] out string appId,
            out int pinningPrevented, out int explicitAppId, out int embeddedShortcutValid);
    }

    public static class Resolver {
        public static string ForWindow(IntPtr hwnd, out bool isExplicit) {
            Type type = Type.GetTypeFromCLSID(new Guid("660B90C8-73A9-4B58-8CAE-355B7F55341B"));
            IApplicationResolver resolver = (IApplicationResolver)Activator.CreateInstance(type);
            string appId;
            int pinningPrevented, explicitAppId, embeddedShortcutValid;
            int result = resolver.GetAppIDForWindow(hwnd, out appId, out pinningPrevented, out explicitAppId, out embeddedShortcutValid);
            if (result < 0) Marshal.ThrowExceptionForHR(result);
            isExplicit = explicitAppId != 0;
            return appId;
        }
    }
}
'@

$explicit = $false
$id = [TouchDeckEvidence.Resolver]::ForWindow([IntPtr]$Hwnd, [ref]$explicit)
[ordered]@{ appUserModelId = $id; explicit = $explicit } | ConvertTo-Json -Compress
