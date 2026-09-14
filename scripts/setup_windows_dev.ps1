# Install the Windows build toolchain for OpenMango. Run in an administrator PowerShell:
#   powershell -ExecutionPolicy Bypass -File scripts\setup_windows_dev.ps1
$ErrorActionPreference = 'Stop'

# Rust target directories nest deeply; allow paths longer than 260 characters.
Set-ItemProperty -Path 'HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem' -Name LongPathsEnabled -Value 1

$packages = @(
    @{ Id = 'Git.Git' },
    # Vendored OpenSSL needs a complete perl; Git's bundled perl is not enough.
    @{ Id = 'StrawberryPerl.StrawberryPerl' },
    @{ Id = 'Kitware.CMake' },
    @{ Id = 'LLVM.LLVM' },
    @{ Id = 'NASM.NASM' },
    @{ Id = 'Python.Python.3.13' },
    @{ Id = 'Rustlang.Rustup' },
    @{ Id = 'Microsoft.VisualStudio.2022.BuildTools'; Override = '--wait --quiet --norestart --nocache --add Microsoft.VisualStudio.Workload.VCTools --add Microsoft.VisualStudio.Component.VC.Tools.ARM64 --includeRecommended' }
)
foreach ($package in $packages) {
    # The msstore source can fail certificate checks in virtual machines; these all come from winget.
    $arguments = @('install', '-e', '--id', $package.Id, '--source', 'winget', '--silent', '--accept-package-agreements', '--accept-source-agreements', '--disable-interactivity')
    if ($package.Override) { $arguments += @('--override', $package.Override) }
    & winget @arguments
    Write-Host "$($package.Id): exit $LASTEXITCODE"
}

# Bun pinned to the version CI uses.
& powershell -NoProfile -ExecutionPolicy Bypass -Command "& ([scriptblock]::Create((irm https://bun.sh/install.ps1))) -Version 1.4.2"
Write-Host 'Open a new terminal so PATH changes apply.'
