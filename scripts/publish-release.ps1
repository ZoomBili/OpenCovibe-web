[CmdletBinding()]
param(
    [string]$Repository = "ZoomBili/OpenCovibe-web",
    [string]$Version = "",
    [string]$ZigPath = "",
    [string]$CargoPath = "",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$ManifestPath = Join-Path $Root "src-tauri\Cargo.toml"
$DistPath = Join-Path $Root "dist-release"
$Toolchain = "stable-x86_64-pc-windows-gnu"

function Write-Step([string]$Message) {
    Write-Host "[OpenCovibe] $Message"
}

function Resolve-Executable([string]$ExplicitPath, [string]$Name) {
    if ($ExplicitPath) {
        return (Resolve-Path $ExplicitPath).Path
    }
    $command = Get-Command $Name -ErrorAction SilentlyContinue
    if ($command) {
        return $command.Source
    }
    return ""
}

function Write-Utf8NoBom([string]$Path, [string]$Content) {
    [System.IO.File]::WriteAllText($Path, $Content, [System.Text.UTF8Encoding]::new($false))
}

function Get-GitHubToken {
    $credentialLines = "protocol=https`nhost=github.com`n`n" | git credential fill
    if ($LASTEXITCODE -ne 0) {
        throw "Git Credential Manager has no usable GitHub login. Run: git credential-manager github login --browser"
    }

    $credential = @{}
    foreach ($line in $credentialLines) {
        $separator = $line.IndexOf("=")
        if ($separator -gt 0) {
            $credential[$line.Substring(0, $separator)] = $line.Substring($separator + 1)
        }
    }
    if (-not $credential.ContainsKey("password")) {
        throw "Git Credential Manager returned no GitHub token"
    }
    return $credential["password"]
}

function Invoke-GitHubApi {
    param(
        [Parameter(Mandatory)] [string]$Uri,
        [Parameter(Mandatory)] [hashtable]$Headers,
        [string]$Method = "Get",
        [object]$Body = $null
    )

    $arguments = @{
        Uri = $Uri
        Headers = $Headers
        Method = $Method
    }
    if ($null -ne $Body) {
        $arguments.Body = $Body | ConvertTo-Json -Depth 8
        $arguments.ContentType = "application/json"
    }
    Invoke-RestMethod @arguments
}

if (-not $Version) {
    $packageJson = Get-Content (Join-Path $Root "package.json") -Raw | ConvertFrom-Json
    $Version = "v$($packageJson.version)"
} elseif (-not $Version.StartsWith("v")) {
    $Version = "v$Version"
}

if ($Repository -notmatch "^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$") {
    throw "Invalid GitHub repository: $Repository"
}

if (-not $ZigPath) {
    $ZigPath = Resolve-Executable "" "zig"
}
if (-not $ZigPath) {
    $bundledZig = Join-Path $env:TEMP "opencovibe-zig\zig-x86_64-windows-0.14.1\zig.exe"
    if (Test-Path $bundledZig) {
        $ZigPath = $bundledZig
    }
}
if (-not $ZigPath -or -not (Test-Path $ZigPath)) {
    throw "Zig is required. Install Zig 0.14+ or pass -ZigPath C:\path\to\zig.exe"
}
$ZigPath = (Resolve-Path $ZigPath).Path

$CargoPath = Resolve-Executable $CargoPath "cargo"
if (-not $CargoPath) {
    $temporaryCargo = Join-Path $env:TEMP "opencovibe-rust-toolchain\cargo\bin\cargo.exe"
    if (Test-Path $temporaryCargo) {
        $CargoPath = $temporaryCargo
    }
}
if (-not $CargoPath) {
    throw "Cargo is required. Install Rust or pass -CargoPath C:\path\to\cargo.exe"
}
$CargoPath = (Resolve-Path $CargoPath).Path
$RustupPath = Join-Path (Split-Path $CargoPath) "rustup.exe"
if (-not (Test-Path $RustupPath)) {
    throw "rustup.exe was not found next to cargo.exe"
}

$temporaryToolRoot = Join-Path $env:TEMP "opencovibe-rust-toolchain"
$temporaryCargoHome = Join-Path $temporaryToolRoot "cargo"
if ($CargoPath.StartsWith($temporaryCargoHome, [StringComparison]::OrdinalIgnoreCase)) {
    $env:CARGO_HOME = $temporaryCargoHome
    $env:RUSTUP_HOME = Join-Path $temporaryToolRoot "rustup"
    Write-Step "Using temporary Rust toolchain at $temporaryToolRoot"
}

$ToolsPath = Join-Path $env:TEMP "opencovibe-release-tools"
New-Item -ItemType Directory -Force -Path $ToolsPath | Out-Null

$targets = @(
    @{ RustTarget = "x86_64-unknown-linux-gnu"; ZigTarget = "x86_64-linux-gnu"; Arch = "amd64" },
    @{ RustTarget = "aarch64-unknown-linux-gnu"; ZigTarget = "aarch64-linux-gnu"; Arch = "arm64" }
)

foreach ($target in $targets) {
    $ccPath = Join-Path $ToolsPath "zig-$($target.Arch)-cc.cmd"
    $arPath = Join-Path $ToolsPath "zig-$($target.Arch)-ar.cmd"
    $ccScript = @"
@echo off
setlocal EnableDelayedExpansion
set "ARGS="
:loop
if "%~1"=="" goto run
set "ARG=%~1"
if /I "!ARG:~0,9!"=="--target=" (
  shift
  goto loop
)
set ARGS=!ARGS! "%~1"
shift
goto loop
:run
"$ZigPath" cc -target $($target.ZigTarget) !ARGS!
"@
    Write-Utf8NoBom $ccPath $ccScript
    Write-Utf8NoBom $arPath "@echo off`r`n`"$ZigPath`" ar %*`r`n"
    $target.CcPath = $ccPath
    $target.ArPath = $arPath
}

Push-Location $Root
try {
    if (-not $SkipBuild) {
        Write-Step "Building embedded frontend"
        npm run build
        if ($LASTEXITCODE -ne 0) { throw "Frontend build failed" }

        Write-Step "Checking Rust GNU toolchain"
        $installedToolchains = @(& $RustupPath toolchain list)
        if ($LASTEXITCODE -ne 0) { throw "Failed to inspect installed Rust toolchains" }
        $toolchainPattern = "^$([Regex]::Escape($Toolchain))(\s|$)"
        if (-not ($installedToolchains | Where-Object { $_ -match $toolchainPattern })) {
            Write-Step "Installing Rust GNU toolchain"
            & $RustupPath toolchain install $Toolchain --profile minimal
            if ($LASTEXITCODE -ne 0) { throw "Rust GNU toolchain installation failed" }
        }

        foreach ($target in $targets) {
            $installedTargets = @(& $RustupPath target list --installed --toolchain $Toolchain)
            if ($LASTEXITCODE -ne 0) { throw "Failed to inspect installed Rust targets" }
            if ($target.RustTarget -notin $installedTargets) {
                Write-Step "Installing Rust target $($target.RustTarget)"
                & $RustupPath target add $target.RustTarget --toolchain $Toolchain
                if ($LASTEXITCODE -ne 0) { throw "Failed to install Rust target $($target.RustTarget)" }
            }

            Write-Step "Building Linux/$($target.Arch)"

            $targetKey = $target.RustTarget.Replace("-", "_")
            Set-Item "env:CC_$targetKey" $target.CcPath
            Set-Item "env:AR_$targetKey" $target.ArPath
            Set-Item "env:CARGO_TARGET_$($targetKey.ToUpperInvariant())_LINKER" $target.CcPath

            & $CargoPath "+$Toolchain" build --release --locked --target $target.RustTarget --manifest-path $ManifestPath
            if ($LASTEXITCODE -ne 0) { throw "Rust build failed for $($target.RustTarget)" }
        }
    }

    New-Item -ItemType Directory -Force -Path $DistPath | Out-Null
    $assets = @()
    foreach ($target in $targets) {
        $packageName = "opencovibe-server_${Version}_linux_$($target.Arch)"
        $packagePath = Join-Path $DistPath $packageName
        $archivePath = Join-Path $DistPath "$packageName.tar.gz"
        $checksumPath = "$archivePath.sha256"

        foreach ($path in @($packagePath, $archivePath, $checksumPath)) {
            if (Test-Path $path) {
                Remove-Item -Recurse -Force -LiteralPath $path
            }
        }

        New-Item -ItemType Directory -Path $packagePath | Out-Null
        $binaryPath = Join-Path $Root "src-tauri\target\$($target.RustTarget)\release\opencovibe-server"
        if (-not (Test-Path $binaryPath)) {
            throw "Missing release binary: $binaryPath"
        }
        Copy-Item $binaryPath (Join-Path $packagePath "opencovibe-server")
        Copy-Item (Join-Path $Root "README.md") $packagePath

        tar.exe -C $DistPath -czf $archivePath $packageName
        if ($LASTEXITCODE -ne 0) { throw "Failed to create $archivePath" }
        $hash = (Get-FileHash -Algorithm SHA256 $archivePath).Hash.ToLowerInvariant()
        Write-Utf8NoBom $checksumPath "$hash  $([System.IO.Path]::GetFileName($archivePath))`n"
        $assets += $archivePath, $checksumPath
    }

    Write-Step "Building fnOS package"
    npm run fnos:package -- --version $Version --output $DistPath
    if ($LASTEXITCODE -ne 0) { throw "fnOS package build failed" }
    $releaseVersion = $Version.TrimStart("v")
    $fnosPackagePath = Join-Path $DistPath "opencovibe-web_v$releaseVersion.fpk"
    $fnosChecksumPath = "$fnosPackagePath.sha256"
    if (-not (Test-Path $fnosPackagePath) -or -not (Test-Path $fnosChecksumPath)) {
        throw "fnOS package assets were not generated"
    }
    $assets += $fnosPackagePath, $fnosChecksumPath

    Write-Step "Publishing $Version to $Repository"
    $token = Get-GitHubToken
    $headers = @{
        Authorization = "Bearer $token"
        Accept = "application/vnd.github+json"
        "X-GitHub-Api-Version" = "2022-11-28"
        "User-Agent" = "OpenCovibe-local-release"
    }
    $release = $null
    try {
        $release = Invoke-GitHubApi -Uri "https://api.github.com/repos/$Repository/releases/tags/$Version" -Headers $headers
    } catch {
        $statusCode = $_.Exception.Response.StatusCode.value__
        if ($statusCode -ne 404) { throw }
    }
    if (-not $release) {
        $release = Invoke-GitHubApi -Uri "https://api.github.com/repos/$Repository/releases" -Headers $headers -Method Post -Body @{
            tag_name = $Version
            target_commitish = "main"
            name = "OpenCovibe Web Server $Version"
            generate_release_notes = $true
            draft = $false
            prerelease = $false
        }
    }

    $uploadBase = $release.upload_url -replace "\{.*$", ""
    foreach ($assetPath in $assets) {
        $assetName = [System.IO.Path]::GetFileName($assetPath)
        $existingAsset = $release.assets | Where-Object { $_.name -eq $assetName } | Select-Object -First 1
        if ($existingAsset) {
            Invoke-GitHubApi -Uri "https://api.github.com/repos/$Repository/releases/assets/$($existingAsset.id)" -Headers $headers -Method Delete | Out-Null
        }
        Write-Step "Uploading $assetName"
        $encodedName = [Uri]::EscapeDataString($assetName)
        Invoke-RestMethod -Uri "${uploadBase}?name=$encodedName" -Headers $headers -Method Post -InFile $assetPath -ContentType "application/octet-stream" | Out-Null
    }

    Write-Step "Release published: $($release.html_url)"
} finally {
    Pop-Location
}
