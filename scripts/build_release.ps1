<#
.SYNOPSIS
    Build de release do mx-player para Windows 7 SP1 64-bit.

.DESCRIPTION
    Compila o mx-player para o target x86_64-pc-windows-gnu com otimizacoes
    de release e monta o pacote final de distribuicao contendo:
      - mx-player.exe
      - mpv-1.dll

    Prerequisitos:
      - Rust com target x86_64-pc-windows-gnu instalado
      - Variavel MPV_LIB_PATH apontando para diretorio com libmpv.dll.a
      - mpv-1.dll acessivel em MPV_LIB_PATH ou PATH

    Compatibilidade Win7 SP1 64-bit:
      - Binary compilado com toolchain MinGW (msvcrt, sem ucrt)
      - mpv deve ser versao <= 0.36 sem dependencia de ucrt
      - tokio travado em =1.14.0 (Win7 nao tem GetSystemTimePreciseAsFileTime)

.PARAMETER OutputDir
    Diretorio de saida para o pacote de release.
    Padrao: <raiz-do-projeto>\dist\

.PARAMETER SkipTests
    Se definido, pula a execucao dos testes antes do build.

.PARAMETER CreateZip
    Se definido, cria um arquivo .zip do pacote de release.

.EXAMPLE
    .\scripts\build_release.ps1

.EXAMPLE
    .\scripts\build_release.ps1 -OutputDir "C:\release\mx-player" -CreateZip

.EXAMPLE
    .\scripts\build_release.ps1 -SkipTests -CreateZip
#>

[CmdletBinding()]
param(
    [string]$OutputDir = '',
    [switch]$SkipTests,
    [switch]$CreateZip
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# ──────────────────────────────────────────────
#  Constantes
# ──────────────────────────────────────────────
$TARGET       = 'x86_64-pc-windows-gnu'
$BINARY_NAME  = 'mx-player.exe'
$DLL_NAME     = 'mpv-1.dll'

# ──────────────────────────────────────────────
#  Helpers
# ──────────────────────────────────────────────
$script:stepCount  = 0
$script:errorCount = 0

function Write-Step {
    param([string]$Message)
    $script:stepCount++
    Write-Host ''
    Write-Host "[$($script:stepCount)] $Message" -ForegroundColor Cyan
}

function Write-Ok {
    param([string]$Message)
    Write-Host "    OK  $Message" -ForegroundColor Green
}

function Write-Warn {
    param([string]$Message)
    Write-Host "  WARN  $Message" -ForegroundColor Yellow
}

function Write-Err {
    param([string]$Message)
    Write-Host " ERROR  $Message" -ForegroundColor Red
    $script:errorCount++
}

function Format-FileSize {
    param([long]$Bytes)
    if ($Bytes -ge 1048576) { return "$([math]::Round($Bytes/1048576,1)) MB" }
    return "$([math]::Round($Bytes/1024,1)) KB"
}

function Assert-Command {
    param([string]$Name, [string]$Hint = '')
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        $msg = "Comando '$Name' nao encontrado no PATH."
        if ($Hint) { $msg += " $Hint" }
        Write-Err $msg
        return $false
    }
    return $true
}

# ──────────────────────────────────────────────
#  Determinar raiz do projeto
# ──────────────────────────────────────────────
$ScriptDir   = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Split-Path -Parent $ScriptDir

if (-not (Test-Path (Join-Path $ProjectRoot 'Cargo.toml'))) {
    Write-Host "ERRO: Cargo.toml nao encontrado em '$ProjectRoot'." -ForegroundColor Red
    Write-Host "      Execute este script a partir da raiz do projeto ou de scripts/." -ForegroundColor Red
    exit 1
}

if (-not $OutputDir) {
    $OutputDir = Join-Path $ProjectRoot 'dist'
}

# ──────────────────────────────────────────────
#  Banner
# ──────────────────────────────────────────────
Write-Host ''
Write-Host '==========================================' -ForegroundColor White
Write-Host '  mx-player -- Build de Release (Win7)   ' -ForegroundColor White
Write-Host '==========================================' -ForegroundColor White
Write-Host "  Projeto : $ProjectRoot"
Write-Host "  Target  : $TARGET"
Write-Host "  Saida   : $OutputDir"
Write-Host "  Data    : $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')"

# ──────────────────────────────────────────────
#  Passo 1: Verificar prerequisitos
# ──────────────────────────────────────────────
Write-Step 'Verificando prerequisitos'

$ok = $true
$ok = (Assert-Command 'cargo' 'Instale Rust em https://rustup.rs') -and $ok
$ok = (Assert-Command 'rustup') -and $ok

if ($ok) {
    # Verificar target instalado
    $targets = (& rustup target list --installed 2>&1) | Where-Object { $_ -match $TARGET }
    if ($targets) {
        Write-Ok "Target $TARGET instalado."
    } else {
        Write-Err "Target $TARGET nao instalado. Execute: rustup target add $TARGET"
        $ok = $false
    }
}

# Localizar mpv-1.dll (necessario para bundling)
$mpvDllPath = $null

$mpvLibPath = $env:MPV_LIB_PATH
if ($mpvLibPath -and (Test-Path $mpvLibPath)) {
    $candidate = Join-Path $mpvLibPath $DLL_NAME
    if (Test-Path $candidate) {
        $mpvDllPath = $candidate
        Write-Ok "mpv-1.dll encontrado via MPV_LIB_PATH: $mpvDllPath"
    }
}

if (-not $mpvDllPath) {
    # Tentar encontrar no PATH
    $found = Get-Command $DLL_NAME -ErrorAction SilentlyContinue
    if ($found) {
        $mpvDllPath = $found.Source
        Write-Ok "mpv-1.dll encontrado no PATH: $mpvDllPath"
    }
}

if (-not $mpvDllPath) {
    # Tentar caminhos convencionais
    $fallbackPaths = @(
        'C:\mpv-dev\lib64\mpv-1.dll',
        'C:\mpv-dev\lib\mpv-1.dll',
        (Join-Path $ProjectRoot 'mpv-dev\mpv-1.dll'),
        (Join-Path $ProjectRoot 'mpv-1.dll')
    )
    foreach ($p in $fallbackPaths) {
        if (Test-Path $p) {
            $mpvDllPath = $p
            Write-Ok "mpv-1.dll encontrado (fallback): $mpvDllPath"
            break
        }
    }
}

if (-not $mpvDllPath) {
    Write-Warn "mpv-1.dll NAO encontrado. O pacote de release nao incluira a DLL."
    Write-Warn "Defina MPV_LIB_PATH ou coloque mpv-1.dll na raiz do projeto."
    Write-Warn "O binario exige mpv-1.dll no mesmo diretorio para funcionar."
} else {
    # Verificar dependencia de ucrt (incompativel com Win7)
    $dumpbinCmd = Get-Command 'dumpbin' -ErrorAction SilentlyContinue
    $objdumpCmd = Get-Command 'x86_64-w64-mingw32-objdump' -ErrorAction SilentlyContinue

    if ($objdumpCmd) {
        $imports = (& x86_64-w64-mingw32-objdump -p $mpvDllPath 2>&1) | Out-String
        if ($imports -match 'ucrtbase') {
            Write-Warn "ATENCAO: mpv-1.dll depende de 'ucrtbase' (ucrt) — incompativel com Win7!"
            Write-Warn "Use binarios MinGW (msvcrt) de https://sourceforge.net/projects/mpv-player-windows/files/libmpv/"
        } else {
            Write-Ok "mpv-1.dll nao depende de ucrt — compativel com Win7."
        }
    } else {
        Write-Warn "objdump MinGW nao encontrado — verificacao de ucrt pulada."
        Write-Warn "Confirme manualmente que mpv-1.dll usa msvcrt (nao ucrt)."
    }

    # Exibir tamanho da DLL
    $dllSize = (Get-Item $mpvDllPath).Length
    Write-Ok "Tamanho mpv-1.dll: $(Format-FileSize $dllSize)"
}

if (-not $ok) {
    Write-Host ''
    Write-Host 'ERRO: Pre-requisitos nao atendidos. Corrija os erros acima e tente novamente.' -ForegroundColor Red
    Write-Host 'Execute scripts\check_env.ps1 para diagnostico completo.' -ForegroundColor Yellow
    exit 1
}

# ──────────────────────────────────────────────
#  Passo 2: Executar testes
# ──────────────────────────────────────────────
if (-not $SkipTests) {
    Write-Step 'Executando testes unitarios'
    Push-Location $ProjectRoot
    try {
        Write-Host '    Executando: cargo test' -ForegroundColor DarkGray
        & cargo test 2>&1 | ForEach-Object {
            Write-Host "    $_" -ForegroundColor DarkGray
        }
        if ($LASTEXITCODE -ne 0) {
            Write-Err "cargo test falhou (exit code $LASTEXITCODE)."
            Write-Host 'Use -SkipTests para pular os testes (nao recomendado em release).' -ForegroundColor Yellow
            exit 1
        }
        Write-Ok 'Todos os testes passaram.'
    } finally {
        Pop-Location
    }
} else {
    Write-Warn 'Testes pulados (--SkipTests ativado).'
}

# ──────────────────────────────────────────────
#  Passo 3: Compilar release
# ──────────────────────────────────────────────
Write-Step "Compilando release para $TARGET"

Push-Location $ProjectRoot
try {
    $buildArgs = @('build', '--release', '--target', $TARGET)
    Write-Host "    Executando: cargo $($buildArgs -join ' ')" -ForegroundColor DarkGray
    Write-Host '    Isso pode levar alguns minutos na primeira execucao...' -ForegroundColor DarkGray
    Write-Host ''

    $buildStart = Get-Date
    & cargo @buildArgs
    $buildExit = $LASTEXITCODE
    $buildElapsed = [math]::Round(((Get-Date) - $buildStart).TotalSeconds, 1)

    if ($buildExit -ne 0) {
        Write-Err "cargo build falhou (exit code $buildExit) apos ${buildElapsed}s."
        Write-Host ''
        Write-Host 'Dicas de troubleshooting:' -ForegroundColor Yellow
        Write-Host '  - Verifique MPV_LIB_PATH (deve apontar para diretorio com libmpv.dll.a)' -ForegroundColor Yellow
        Write-Host '  - Execute scripts\check_env.ps1 para diagnostico completo' -ForegroundColor Yellow
        Write-Host '  - Confirme que o toolchain MinGW esta no PATH' -ForegroundColor Yellow
        exit 1
    }

    Write-Ok "Build concluido em ${buildElapsed}s."
} finally {
    Pop-Location
}

# ──────────────────────────────────────────────
#  Passo 4: Localizar binario gerado
# ──────────────────────────────────────────────
Write-Step 'Localizando binario gerado'

$binarySource = Join-Path $ProjectRoot "target\$TARGET\release\$BINARY_NAME"
if (-not (Test-Path $binarySource)) {
    Write-Err "Binario nao encontrado em: $binarySource"
    exit 1
}

$binSize = (Get-Item $binarySource).Length
Write-Ok "Binario: $binarySource ($(Format-FileSize $binSize))"

# ──────────────────────────────────────────────
#  Passo 5: Montar pacote de release
# ──────────────────────────────────────────────
Write-Step 'Montando pacote de release'

# Criar diretorio de saida (limpar se existir)
if (Test-Path $OutputDir) {
    Write-Warn "Diretorio de saida ja existe — limpando: $OutputDir"
    Remove-Item $OutputDir -Recurse -Force
}
New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
Write-Ok "Diretorio criado: $OutputDir"

# Copiar executavel
$binaryDest = Join-Path $OutputDir $BINARY_NAME
Copy-Item $binarySource $binaryDest
Write-Ok "Copiado: $BINARY_NAME ($(Format-FileSize (Get-Item $binaryDest).Length))"

# Copiar mpv-1.dll (obrigatoria para execucao)
if ($mpvDllPath) {
    $dllDest = Join-Path $OutputDir $DLL_NAME
    Copy-Item $mpvDllPath $dllDest
    Write-Ok "Copiado: $DLL_NAME ($(Format-FileSize (Get-Item $dllDest).Length))"
} else {
    Write-Warn "mpv-1.dll nao incluida no pacote — copie manualmente antes de distribuir."
}

# Copiar data/ (fontes de stream de exemplo)
$dataSource = Join-Path $ProjectRoot 'data'
if (Test-Path $dataSource) {
    $dataDest = Join-Path $OutputDir 'data'
    Copy-Item $dataSource $dataDest -Recurse
    Write-Ok "Copiado: data/ (arquivos de exemplo de fontes)"
}

# ──────────────────────────────────────────────
#  Passo 6: Gerar README de release
# ──────────────────────────────────────────────
Write-Step 'Gerando README de release'

$readmeContent = @"
# MX Player — Release

## Conteudo do Pacote

  mx-player.exe   Player de video (streams HLS/RTSP/HTTP)
  mpv-1.dll       Biblioteca de decodificacao de video (obrigatoria)
  data/           Arquivos de exemplo de fontes de stream

## Requisitos de Sistema

  - Windows 7 SP1 64-bit ou superior
  - Resolucao minima: 800x600
  - Conexao de rede para acesso aos streams

## Uso

  1. Coloque mx-player.exe e mpv-1.dll no mesmo diretorio.
  2. Execute mx-player.exe para abrir a interface grafica, ou:

     mx-player.exe --list                    Lista todos os endpoints
     mx-player.exe --endpoint "Nome Canal"   Abre stream diretamente

  3. Edite data\sources.csv ou data\sources.example.json para
     configurar seus proprios endpoints de stream.

## Arquivo de Estado

  O arquivo state.json e criado automaticamente no diretorio de
  trabalho e persiste o ultimo status conhecido dos endpoints entre
  sessoes.

## Log

  Defina a variavel de ambiente RUST_LOG=info para ativar logging.
  Exemplo: set RUST_LOG=info && mx-player.exe

## Compatibilidade

  Binario compilado para x86_64-pc-windows-gnu (MinGW/msvcrt).
  Compativel com Windows 7 SP1 64-bit sem instalacao adicional.
  NAO requer Visual C++ Redistributable ou ucrt.

## Versao

  Build: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')
  Target: $TARGET
"@

$readmePath = Join-Path $OutputDir 'README.txt'
Set-Content -Path $readmePath -Value $readmeContent -Encoding UTF8
Write-Ok "README.txt criado."

# ──────────────────────────────────────────────
#  Passo 7: Criar ZIP (opcional)
# ──────────────────────────────────────────────
if ($CreateZip) {
    Write-Step 'Criando arquivo ZIP'

    $zipName = "mx-player-win7-x64-$(Get-Date -Format 'yyyyMMdd').zip"
    $zipPath = Join-Path (Split-Path -Parent $OutputDir) $zipName

    if (Test-Path $zipPath) {
        Remove-Item $zipPath -Force
    }

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::CreateFromDirectory($OutputDir, $zipPath)

    $zipSize = (Get-Item $zipPath).Length
    Write-Ok "ZIP criado: $zipPath ($(Format-FileSize $zipSize))"
}

# ──────────────────────────────────────────────
#  Sumario final
# ──────────────────────────────────────────────
Write-Host ''
Write-Host '==========================================' -ForegroundColor White
Write-Host '  Build de Release Concluido             ' -ForegroundColor Green
Write-Host '==========================================' -ForegroundColor White
Write-Host ''
Write-Host "  Pacote : $OutputDir" -ForegroundColor White

Get-ChildItem $OutputDir -Recurse -File | ForEach-Object {
    $rel = $_.FullName.Substring($OutputDir.Length + 1)
    $sz  = Format-FileSize $_.Length
    Write-Host "    $rel  ($sz)" -ForegroundColor Gray
}

Write-Host ''
Write-Host '  Instrucoes de validacao (Win7 SP1 64-bit):' -ForegroundColor White
Write-Host '    1. Copie o pacote para a VM Windows 7 SP1 64-bit.' -ForegroundColor Gray
Write-Host '    2. Confirme que mx-player.exe e mpv-1.dll estao no mesmo diretorio.' -ForegroundColor Gray
Write-Host '    3. Execute: mx-player.exe --list' -ForegroundColor Gray
Write-Host '       Esperado: listagem dos endpoints sem erros de DLL.' -ForegroundColor Gray
Write-Host '    4. Execute: mx-player.exe --endpoint "nome-do-endpoint"' -ForegroundColor Gray
Write-Host '       Esperado: stream iniciado com hwdec=auto (DXVA2).' -ForegroundColor Gray
Write-Host '    5. Feche a janela e verifique que state.json foi criado.' -ForegroundColor Gray
Write-Host '    6. Desabilite a URL primaria no firewall e confirme fallback para' -ForegroundColor Gray
Write-Host '       URL secundaria (status "Degraded" no health check).' -ForegroundColor Gray
Write-Host ''

if ($script:errorCount -gt 0) {
    Write-Host "  AVISO: $($script:errorCount) erro(s) encontrado(s) durante o processo." -ForegroundColor Yellow
    exit 1
}

Write-Host '  Pacote pronto para distribuicao e teste em Win7!' -ForegroundColor Green
Write-Host ''
exit 0
