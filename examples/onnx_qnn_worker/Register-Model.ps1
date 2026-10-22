<#
.SYNOPSIS
    Registers an ONNX model into the PhotoCraft QNN Worker package.

.DESCRIPTION
    PhotoCraft's compatibility adapter engine verifies all files against adapter.json.
    This script:
    1. Copies the specified .onnx model into the package directory (under models/).
    2. Computes the SHA256 digest of the model.
    3. Adds the model profile to qnn-models.json and sets it as defaultModel.
    4. Updates adapter.json to include the model in the package files list.

.EXAMPLE
    .\Register-Model.ps1 -ModelPath .\u2netp.onnx -InputName "input.1" -OutputName "1959" -Width 320 -Height 320
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ModelPath,

    [string]$Id = "default-subject",
    [string]$InputName = "input",
    [int]$Width = 512,
    [int]$Height = 512,
    [string]$OutputName = "output",
    [ValidateSet("probability", "sigmoid")]
    [string]$Postprocess = "probability"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scriptDir = $PSScriptRoot
$resolvedModelPath = Resolve-Path -LiteralPath $ModelPath -ErrorAction Stop

# 1. Target directory
$modelsDir = Join-Path $scriptDir "models"
if (-not (Test-Path -LiteralPath $modelsDir)) {
    New-Item -ItemType Directory -Path $modelsDir | Out-Null
}

$modelFileName = [System.IO.Path]::GetFileName($resolvedModelPath)
$targetModelPath = Join-Path $modelsDir $modelFileName
Copy-Item -LiteralPath $resolvedModelPath -Destination $targetModelPath -Force

# 2. Compute SHA256
$hash = (Get-FileHash -LiteralPath $targetModelPath -Algorithm SHA256).Hash.ToLowerInvariant()
Write-Host "Model: $modelFileName"
Write-Host "SHA256: $hash"

# 3. Update qnn-models.json
$configPath = Join-Path $scriptDir "qnn-models.json"
$config = if (Test-Path -LiteralPath $configPath) {
    Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json -AsHashtable
} else {
    @{ schemaVersion = 1; defaultModel = $null; profiles = @() }
}

$relativeModelPath = "models/$modelFileName"
$newProfile = [ordered]@{
    id = $Id
    operation = "select-subject"
    model = $relativeModelPath
    sha256 = $hash
    input = [ordered]@{
        name = $InputName
        layout = "nchw"
        shape = @(1, 3, $Height, $Width)
        mean = @(0.485, 0.456, 0.406)
        scale = @(4.367, 4.464, 4.444)
        resize = "bilinear"
        colorPolicy = "assume-srgb"
        alphaPolicy = "ignore-straight"
    }
    output = [ordered]@{
        name = $OutputName
        layout = "nchw"
        shape = @(1, 1, $Height, $Width)
        postprocess = $Postprocess
        resize = "bilinear"
    }
}

# Replace existing profile with same ID or add
$existingProfiles = @($config.profiles | Where-Object { $_.id -ne $Id })
$config.profiles = $existingProfiles + $newProfile
$config.defaultModel = $Id

[System.IO.File]::WriteAllText($configPath, ($config | ConvertTo-Json -Depth 10), [System.Text.UTF8Encoding]::new($false))
Write-Host "Updated $configPath with defaultModel: $Id"

# 4. Update adapter.json
$adapterPath = Join-Path $scriptDir "adapter.json"
if (Test-Path -LiteralPath $adapterPath) {
    $adapter = Get-Content -LiteralPath $adapterPath -Raw | ConvertFrom-Json -AsHashtable
    $fileList = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($f in $adapter.files) { $fileList.Add($f) | Out-Null }
    $fileList.Add($relativeModelPath) | Out-Null
    $adapter.files = @($fileList | Sort-Object)
    [System.IO.File]::WriteAllText($adapterPath, ($adapter | ConvertTo-Json -Depth 15), [System.Text.UTF8Encoding]::new($false))
    Write-Host "Updated $adapterPath to include $relativeModelPath"
}

Write-Host "Model registered successfully! Now open PhotoCraft, go to Filter > Plug-ins > Compatibility Adapters..., and install adapter.json." -ForegroundColor Green
