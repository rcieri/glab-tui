param(
    [string]$Repo = "rcieri/glab-tui",
    [string]$Prefix = "$env:USERPROFILE\.local\bin"
)

$ErrorActionPreference = "Stop"

function Get-AssetName {
    $arch = "amd64"
    if ([Environment]::Is64BitOperatingSystem -eq $false) {
        Write-Error "32-bit Windows is not supported"
        exit 1
    }
    return "glab-tui-windows-$arch.zip"
}

function Get-LatestRelease {
    $url = "https://api.github.com/repos/$Repo/releases/latest"
    $headers = @{}
    if ($env:GITHUB_TOKEN) {
        $headers["Authorization"] = "Bearer $env:GITHUB_TOKEN"
        return Invoke-RestMethod -Uri $url -Headers $headers -UseBasicParsing
    }
    return Invoke-RestMethod -Uri $url -UseBasicParsing
}

function Main {
    $asset = Get-AssetName

    Write-Host "Fetching latest release..."

    $release = Get-LatestRelease
    $tag = $release.tag_name
    $downloadUrl = ($release.assets | Where-Object { $_.name -eq $asset }).browser_download_url
    $checksumUrl = ($release.assets | Where-Object { $_.name -eq "$asset.sha256" }).browser_download_url

    if (-not $downloadUrl) {
        Write-Error "No asset found for $asset"
        exit 1
    }
    if (-not $checksumUrl) {
        Write-Error "Release $tag publishes no $asset.sha256; refusing to install an unverified binary"
        exit 1
    }

    $tmpdir = Join-Path $env:TEMP "glab-tui-install"
    New-Item -ItemType Directory -Force -Path $tmpdir | Out-Null

    try {
        $zipPath = Join-Path $tmpdir $asset
        $checksumPath = "$zipPath.sha256"
        Write-Host "Downloading $asset..."
        Invoke-WebRequest -Uri $downloadUrl -OutFile $zipPath -UseBasicParsing
        Invoke-WebRequest -Uri $checksumUrl -OutFile $checksumPath -UseBasicParsing

        $expected = ((Get-Content -Path $checksumPath -TotalCount 1) -split '\s+')[0]
        $actual = (Get-FileHash -Path $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if (-not $expected -or $actual -ne $expected) {
            throw "Checksum mismatch for ${asset}; refusing to install. Expected sha256 $expected, got $actual"
        }
        Write-Host "Checksum verified: sha256 $actual"

        Write-Host "Extracting..."
        Expand-Archive -Path $zipPath -DestinationPath $tmpdir -Force

        New-Item -ItemType Directory -Force -Path $Prefix | Out-Null
        $binary = "glab-tui.exe"
        Copy-Item (Join-Path $tmpdir $binary) (Join-Path $Prefix $binary) -Force

        Write-Host "Installed glab-tui $tag to $Prefix\$binary"

        $currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
        if ($currentPath -notlike "*$Prefix*") {
            Write-Host "Warning: $Prefix is not in your user PATH."
            Write-Host "Add it manually or run (in PowerShell):"
            Write-Host '  [Environment]::SetEnvironmentVariable("Path", "$env:Path;$Prefix", "User")'
        }
    }
    finally {
        Remove-Item -Recurse -Force $tmpdir -ErrorAction SilentlyContinue
    }
}

Main
