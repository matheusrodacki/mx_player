<#
.SYNOPSIS
    Verifica dependencias do ambiente de build para o mx-player (Windows 7 target).

.DESCRIPTION
    Valida a presenca e versao correta de todas as ferramentas e bibliotecas
    necessarias para compilar o mx-player para x86_64-pc-windows-gnu (Win7 SP1+).

    Verificacoes realizadas:
      1. Rust / rustup instalado
      2. Target x86_64-pc-windows-gnu instalado
      3. Toolchain MinGW (x86_64-w64-mingw32-gcc) no PATH
      4. Variavel MPV_LIB_PATH definida
      5. libmpv.dll.a (import library) em MPV_LIB_PATH
      6. mpv-1.dll acessivel (em MPV_LIB_PATH ou no PATH)
      7. mpv-1.dll NAO depende de ucrt (compatibilidade Win7)
      8. Versao do tokio no Cargo.toml <= 1.14.0
      9. Dependencia libmpv declarada no Cargo.toml

.EXAMPLE
    .\scripts\check_env.ps1

.EXAMPLE
    # Definir MPV_LIB_PATH antes de rodar:
    $env:MPV_LIB_PATH = 'C:\mpv-dev\lib64'
    .\scripts\check_env.ps1
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Continue'

# ──────────────────────────────────────────────
#  Helpers de output
# ──────────────────────────────────────────────
$script:passCount = 0
$script:failCount = 0
$script:warnCount = 0

function Write-Pass {
    param([string]$Message)
    Write-Host "  [PASS] $Message" -ForegroundColor Green
    $script:passCount++
}

function Write-Fail {
    param([string]$Message)
    Write-Host "  [FAIL] $Message" -ForegroundColor Red
    $script:failCount++
}

function Write-Warn {
    param([string]$Message)
    Write-Host "  [WARN] $Message" -ForegroundColor Yellow
    $script:warnCount++
}

function Write-Section {
    param([string]$Title)
    Write-Host ''
    Write-Host "-- $Title" -ForegroundColor Cyan
}

function Format-FileSize {
    param([long]$Bytes)
    if ($Bytes -ge 1048576) {
        $val = [math]::Round($Bytes / 1048576, 1)
        return "$val MB"
    }
    $val = [math]::Round($Bytes / 1024, 1)
    return "$val KB"
}

# ──────────────────────────────────────────────
#  Determinar raiz do projeto (pai de /scripts)
# ──────────────────────────────────────────────
$ScriptDir   = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Split-Path -Parent $ScriptDir
$CargoToml   = Join-Path $ProjectRoot 'Cargo.toml'

Write-Host ''
Write-Host '=============================================' -ForegroundColor White
Write-Host '  mx-player -- Verificacao de Ambiente Build' -ForegroundColor White
Write-Host '=============================================' -ForegroundColor White
Write-Host "  Projeto : $ProjectRoot"
Write-Host "  Data    : $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')"

# ──────────────────────────────────────────────
#  1. Rust / rustup
# ──────────────────────────────────────────────
Write-Section '1. Rust Toolchain'

$rustupCmd = Get-Command 'rustup' -ErrorAction SilentlyContinue
if ($rustupCmd) {
    $rv = (& rustup --version 2>&1 | Select-Object -First 1).ToString()
    Write-Pass "rustup encontrado: $rv"
} else {
    Write-Fail 'rustup NAO encontrado. Instale em https://rustup.rs'
}

$rustcCmd = Get-Command 'rustc' -ErrorAction SilentlyContinue
if ($rustcCmd) {
    $rv = (& rustc --version 2>&1).ToString()
    Write-Pass "rustc encontrado: $rv"
} else {
    Write-Fail 'rustc NAO encontrado no PATH.'
}

$cargoCmd = Get-Command 'cargo' -ErrorAction SilentlyContinue
if ($cargoCmd) {
    $rv = (& cargo --version 2>&1).ToString()
    Write-Pass "cargo encontrado: $rv"
} else {
    Write-Fail 'cargo NAO encontrado no PATH.'
}

# ──────────────────────────────────────────────
#  2. Target x86_64-pc-windows-gnu
# ──────────────────────────────────────────────
Write-Section '2. Target x86_64-pc-windows-gnu'

if ($rustupCmd) {
    $targetList = (& rustup target list --installed 2>&1) -join ' '
    if ($targetList -match 'x86_64-pc-windows-gnu') {
        Write-Pass 'Target x86_64-pc-windows-gnu instalado.'
    } else {
        Write-Fail 'Target x86_64-pc-windows-gnu NAO instalado.'
        Write-Host '         Instale com: rustup target add x86_64-pc-windows-gnu' -ForegroundColor DarkYellow
    }
} else {
    Write-Warn 'Nao foi possivel verificar targets (rustup ausente).'
}

# ──────────────────────────────────────────────
#  3. MinGW toolchain
# ──────────────────────────────────────────────
Write-Section '3. MinGW Toolchain (x86_64-w64-mingw32)'

$mingwGcc = Get-Command 'x86_64-w64-mingw32-gcc' -ErrorAction SilentlyContinue
if ($mingwGcc) {
    $rv = (& x86_64-w64-mingw32-gcc --version 2>&1 | Select-Object -First 1).ToString()
    Write-Pass "x86_64-w64-mingw32-gcc encontrado: $rv"
} else {
    $gccGeneric = Get-Command 'gcc' -ErrorAction SilentlyContinue
    if ($gccGeneric) {
        $rv = (& gcc --version 2>&1 | Select-Object -First 1).ToString()
        if ($rv -match 'mingw|MinGW|w64') {
            Write-Pass "gcc (MinGW) encontrado: $rv"
        } else {
            Write-Warn "gcc encontrado mas pode nao ser MinGW: $rv"
            Write-Host '         Para cross-compile, instale MinGW-w64 e adicione ao PATH.' -ForegroundColor DarkYellow
        }
    } else {
        Write-Fail 'Nenhum compilador MinGW encontrado no PATH.'
        Write-Host '         Instale MinGW-w64: https://www.mingw-w64.org/' -ForegroundColor DarkYellow
        Write-Host '         Ou via MSYS2: pacman -S mingw-w64-x86_64-gcc' -ForegroundColor DarkYellow
    }
}

# ──────────────────────────────────────────────
#  4. MPV_LIB_PATH
# ──────────────────────────────────────────────
Write-Section '4. Variavel de Ambiente MPV_LIB_PATH'

$MpvLibPath = $env:MPV_LIB_PATH
if ($MpvLibPath) {
    Write-Pass "MPV_LIB_PATH definida: $MpvLibPath"
    if (Test-Path $MpvLibPath) {
        Write-Pass 'Diretorio MPV_LIB_PATH existe.'
    } else {
        Write-Fail "Diretorio MPV_LIB_PATH NAO existe: $MpvLibPath"
    }
} else {
    Write-Fail 'MPV_LIB_PATH NAO definida.'
    Write-Host '         Defina antes do build:' -ForegroundColor DarkYellow
    Write-Host "         `$env:MPV_LIB_PATH = 'C:\mpv-dev\lib64'" -ForegroundColor DarkYellow
    Write-Host '         Download: https://sourceforge.net/projects/mpv-player-windows/files/libmpv/' -ForegroundColor DarkYellow
}

# ──────────────────────────────────────────────
#  5. libmpv.dll.a (import library)
# ──────────────────────────────────────────────
Write-Section '5. Import Library libmpv.dll.a'

if ($MpvLibPath -and (Test-Path $MpvLibPath)) {
    $importLib = Join-Path $MpvLibPath 'libmpv.dll.a'
    if (Test-Path $importLib) {
        $sz = Format-FileSize -Bytes (Get-Item $importLib).Length
        Write-Pass "libmpv.dll.a encontrada ($sz): $importLib"
    } else {
        $found = Get-ChildItem -Path $MpvLibPath -Filter 'libmpv*.a' -Recurse -ErrorAction SilentlyContinue
        if ($found) {
            Write-Warn "libmpv.dll.a nao encontrada no root, mas localizada em: $($found.FullName)"
        } else {
            Write-Fail "libmpv.dll.a NAO encontrada em: $MpvLibPath"
            Write-Host '         Verifique se extraiu o pacote mpv-dev corretamente.' -ForegroundColor DarkYellow
            Write-Host '         A import lib deve estar em: [mpv-dev-dir]/lib64/libmpv.dll.a' -ForegroundColor DarkYellow
        }
    }
} else {
    Write-Warn 'Pulando verificacao de libmpv.dll.a (MPV_LIB_PATH ausente ou invalida).'
}

# ──────────────────────────────────────────────
#  6. mpv-1.dll (runtime DLL)
# ──────────────────────────────────────────────
Write-Section '6. Runtime DLL mpv-1.dll'

$mpvDllFound = $false
$mpvDllPath  = $null
$mpvDllName  = 'mpv-1.dll'

# Procurar em MPV_LIB_PATH e diretorios pais
if ($MpvLibPath) {
    $parentDir  = Split-Path $MpvLibPath -Parent
    $binDir     = Join-Path $parentDir 'bin'
    $searchDirs = @($MpvLibPath, $parentDir, $binDir)

    foreach ($sd in $searchDirs) {
        if ($sd -and (Test-Path $sd)) {
            $cand = Join-Path $sd $mpvDllName
            if (Test-Path $cand) {
                $mpvDllFound = $true
                $mpvDllPath  = $cand
                break
            }
        }
    }
}

# Procurar no PATH do sistema
if (-not $mpvDllFound) {
    foreach ($dir in ($env:PATH -split ';')) {
        if ($dir -and (Test-Path $dir -ErrorAction SilentlyContinue)) {
            $cand = Join-Path $dir $mpvDllName
            if (Test-Path $cand) {
                $mpvDllFound = $true
                $mpvDllPath  = $cand
                break
            }
        }
    }
}

if ($mpvDllFound) {
    $sz = Format-FileSize -Bytes (Get-Item $mpvDllPath).Length
    Write-Pass "mpv-1.dll encontrada ($sz): $mpvDllPath"
} else {
    Write-Fail 'mpv-1.dll NAO encontrada.'
    Write-Host '         mpv-1.dll deve estar junto ao .exe no deploy final.' -ForegroundColor DarkYellow
    Write-Host '         Coloque-a tambem em MPV_LIB_PATH para facilitar testes locais.' -ForegroundColor DarkYellow
}

# ──────────────────────────────────────────────
#  7. Verificar dependencia de ucrt (Win7 compat)
# ──────────────────────────────────────────────
Write-Section '7. Compatibilidade Win7 (ucrt em mpv-1.dll)'

if ($mpvDllFound -and $mpvDllPath) {
    $objdump = Get-Command 'x86_64-w64-mingw32-objdump' -ErrorAction SilentlyContinue
    if (-not $objdump) {
        $objdump = Get-Command 'objdump' -ErrorAction SilentlyContinue
    }

    if ($objdump) {
        $dumpOutput = & $objdump.Source '-p' $mpvDllPath 2>&1
        $ucrtRef    = $dumpOutput | Select-String 'ucrtbase|api-ms-win-crt'
        if ($ucrtRef) {
            Write-Fail 'mpv-1.dll DEPENDE de ucrt (nao compativel com Win7 SP1):'
            $ucrtRef | ForEach-Object { Write-Host "           $_" -ForegroundColor Red }
            Write-Host '         Use binarios MinGW com msvcrt, nao ucrt.' -ForegroundColor DarkYellow
            Write-Host '         Tente versoes mais antigas do mpv-dev (<= 0.36).' -ForegroundColor DarkYellow
        } else {
            Write-Pass 'mpv-1.dll nao referencia ucrt -- compativel com Win7 SP1.'
        }
    } else {
        Write-Warn 'objdump nao encontrado; nao foi possivel verificar dependencia de ucrt.'
        Write-Host '         Instale MinGW-w64 para habilitar esta verificacao.' -ForegroundColor DarkYellow
    }
} else {
    Write-Warn 'Pulando verificacao de ucrt (mpv-1.dll nao localizada).'
}

# ──────────────────────────────────────────────
#  8. Versao do tokio no Cargo.toml
# ──────────────────────────────────────────────
Write-Section '8. Versao do tokio (deve ser <= 1.14.0 para Win7)'

if (Test-Path $CargoToml) {
    $cargoContent = Get-Content $CargoToml -Raw

    $tokioRaw = $null
    if ($cargoContent -match 'tokio\s*=.*?version\s*=\s*"([^"]+)"') {
        $tokioRaw = $Matches[1]
    } elseif ($cargoContent -match 'tokio\s*=\s*"([^"]+)"') {
        $tokioRaw = $Matches[1]
    }

    if ($tokioRaw) {
        $tokioClean = ($tokioRaw -replace '^[=^~><=\s]+', '').Trim()
        $parts  = $tokioClean.Split('.')
        $major  = [int]($parts[0])
        $minor  = if ($parts.Count -ge 2) { [int]($parts[1]) } else { 0 }

        if ($major -eq 0 -or ($major -eq 1 -and $minor -le 14)) {
            Write-Pass "tokio versao '$tokioRaw' -- compativel com Win7 SP1 (<=1.14.0)."
        } else {
            Write-Fail "tokio versao '$tokioRaw' -- INCOMPATIVEL com Win7 SP1!"
            Write-Host '         Win7 nao suporta GetSystemTimePreciseAsFileTime (tokio >= 1.15).' -ForegroundColor DarkYellow
            Write-Host '         Mantenha: tokio = { version = "=1.14.0", ... }' -ForegroundColor DarkYellow
        }
    } else {
        Write-Warn 'tokio nao encontrado no Cargo.toml (pode ser dependencia transitiva apenas).'
    }
} else {
    Write-Fail "Cargo.toml nao encontrado em: $CargoToml"
}

# ──────────────────────────────────────────────
#  9. Dependencia libmpv no Cargo.toml
# ──────────────────────────────────────────────
Write-Section '9. Dependencia libmpv no Cargo.toml'

if (Test-Path $CargoToml) {
    $cargoContent = Get-Content $CargoToml -Raw
    if ($cargoContent -match 'libmpv\s*=') {
        if ($cargoContent -match 'libmpv\s*=.*?version\s*=\s*"([^"]+)"') {
            $lv = $Matches[1]
            Write-Pass "libmpv declarada no Cargo.toml (versao: $lv)."
        } else {
            Write-Pass 'libmpv declarada no Cargo.toml.'
        }
    } else {
        Write-Fail 'libmpv NAO declarada no Cargo.toml.'
        Write-Host '         Adicione: libmpv = { version = "=2.0.1", default-features = false }' -ForegroundColor DarkYellow
    }
} else {
    Write-Warn 'Cargo.toml nao encontrado; nao foi possivel verificar dependencia.'
}

# ──────────────────────────────────────────────
#  Sumario final
# ──────────────────────────────────────────────
Write-Host ''
Write-Host '=============================================' -ForegroundColor White
Write-Host '  SUMARIO' -ForegroundColor White
Write-Host '=============================================' -ForegroundColor White
Write-Host "  PASS : $($script:passCount)" -ForegroundColor Green
Write-Host "  WARN : $($script:warnCount)" -ForegroundColor Yellow
Write-Host "  FAIL : $($script:failCount)" -ForegroundColor Red
Write-Host ''

if ($script:failCount -eq 0) {
    Write-Host '  Ambiente OK -- pronto para: cargo build --target x86_64-pc-windows-gnu' -ForegroundColor Green
} else {
    Write-Host '  Corrija os itens FAIL antes de tentar o build.' -ForegroundColor Red
    Write-Host ''
    Write-Host '  Referencia rapida de setup:' -ForegroundColor Cyan
    Write-Host '    1. Baixar mpv-dev MinGW: https://sourceforge.net/projects/mpv-player-windows/files/libmpv/'
    Write-Host "    2. Extrair e definir: `$env:MPV_LIB_PATH = 'C:\mpv-dev\lib64'"
    Write-Host '    3. Instalar target   : rustup target add x86_64-pc-windows-gnu'
    Write-Host '    4. Instalar MinGW    : https://www.mingw-w64.org/ ou via MSYS2'
    Write-Host '    5. Build release     : cargo build --release --target x86_64-pc-windows-gnu'
}

Write-Host ''

# Retornar exit code nao-zero se houver falhas (util em CI)
if ($script:failCount -gt 0) {
    exit 1
}
exit 0
