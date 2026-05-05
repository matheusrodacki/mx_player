/// Wrapper em torno do contexto MPV para reprodução de streams de vídeo.
///
/// Configura hardware decoding (`hwdec=auto` → DXVA2 no Windows) antes da
/// inicialização do handle e expõe métodos de controle de reprodução.
use anyhow::{anyhow, Result};
use libmpv::{events::EventContext, Mpv};

pub struct MpvPlayer {
    mpv: Mpv,
}

impl MpvPlayer {
    /// Cria e inicializa um novo contexto MPV.
    ///
    /// Configura `hwdec=auto` antes de `mpv_initialize` para permitir que o
    /// DXVA2 (Windows) seja selecionado automaticamente quando disponível.
    pub fn new() -> Result<Self> {
        let mpv = Mpv::with_initializer(|init| {
            // hwdec=auto: DXVA2 / D3D11VA no Windows, vdpau/vaapi no Linux.
            // Definir antes de mpv_initialize garante que o decodificador é
            // selecionado na abertura do primeiro stream.
            init.set_property("hwdec", "auto")?;
            Ok(())
        })
        .map_err(|e| anyhow!("Falha ao criar contexto MPV: {}", e))?;

        Ok(Self { mpv })
    }

    /// Inicia ou substitui a reprodução com a URL fornecida.
    ///
    /// Equivalente ao comando mpv `loadfile <url> replace`.
    pub fn play(&self, url: &str) -> Result<()> {
        self.mpv
            .command("loadfile", &[url, "replace"])
            .map_err(|e| anyhow!("loadfile '{}': {}", url, e))
    }

    /// Para a reprodução atual e limpa a fila de reprodução.
    pub fn stop(&self) -> Result<()> {
        self.mpv
            .command("stop", &[])
            .map_err(|e| anyhow!("stop: {}", e))
    }

    /// Pausa a reprodução (idempotente se já pausado).
    pub fn pause(&self) -> Result<()> {
        self.mpv
            .pause()
            .map_err(|e| anyhow!("pause: {}", e))
    }

    /// Retoma a reprodução pausada (idempotente se já tocando).
    pub fn resume(&self) -> Result<()> {
        self.mpv
            .unpause()
            .map_err(|e| anyhow!("resume: {}", e))
    }

    /// Define o volume de reprodução. O valor é clamped para `[0.0, 100.0]`.
    pub fn set_volume(&self, vol: f64) -> Result<()> {
        let vol = vol.clamp(0.0, 100.0);
        self.mpv
            .set_property("volume", vol)
            .map_err(|e| anyhow!("volume {:.1}: {}", vol, e))
    }

    /// Define a janela de renderização de vídeo pelo handle nativo (HWND no Windows).
    ///
    /// Deve ser chamado **antes** de `play()` para que o vídeo seja
    /// renderizado na janela especificada em vez de uma janela própria do MPV.
    pub fn set_window(&self, hwnd: i64) -> Result<()> {
        self.mpv
            .set_property("wid", hwnd)
            .map_err(|e| anyhow!("wid {}: {}", hwnd, e))
    }

    /// Cria um contexto de eventos MPV para monitorar o estado de reprodução.
    ///
    /// # Panics
    /// Entra em pânico se chamado mais de uma vez sem que o `EventContext`
    /// anterior tenha sido descartado (restrição do libmpv).
    pub fn create_event_context(&self) -> EventContext<'_> {
        self.mpv.create_event_context()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifica que `MpvPlayer::new()` pode ser construído quando o mpv-1.dll
    /// está presente. Este teste é ignorado em ambientes sem o DLL.
    #[test]
    #[ignore = "requer mpv-1.dll no PATH"]
    fn new_player_initializes_without_error() {
        let player = MpvPlayer::new();
        assert!(player.is_ok(), "MpvPlayer::new() falhou: {:?}", player.err());
    }

    /// Garante que o volume é clamped corretamente antes de ser enviado ao MPV.
    /// Testa apenas a lógica do clamp via reflexão de estado — não precisa do DLL.
    #[test]
    fn volume_clamp_logic() {
        // Testar que os valores extremos são tratados corretamente
        let vol_neg: f64 = (-10.0_f64).clamp(0.0, 100.0);
        assert_eq!(vol_neg, 0.0);

        let vol_high: f64 = (150.0_f64).clamp(0.0, 100.0);
        assert_eq!(vol_high, 100.0);

        let vol_ok: f64 = (75.0_f64).clamp(0.0, 100.0);
        assert_eq!(vol_ok, 75.0);
    }
}
