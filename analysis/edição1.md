 A análise e a correção pontual da ativação do modo de corte foram concluídas com a menor alteração segura possível.                                                       
  ──────                                                                                                                                                                    
  ### 1. Diagnóstico e Respostas aos Pontos Prévios                                                                                                                         
                                                                                                                                                                            
  1. Inspeção de crop.rs, lib.rs e canvas.rs:                                                                                                                               
      • O comando "ui.crop" já estava completamente implementado em crop.rs e registrado em lib.rs:1532, delegando para crop::toggle(app).                                  
      • Ao ser ativado com uma imagem válida, crop::toggle define app.canvas.crop = Some(pos). No ciclo de pintura do canvas (objects.rs:500), crop::paint desenha as 8     
      alças pretas de corte e a área escurecida ao redor.                                                                                                                   
      • O menu de contexto em canvas.rs:2891 chamava erroneamente "ui.tab" com "Picture Format" sob o rótulo "Size and Crop…".                                              
  2. Condições para crop::toggle ativar o corte e tratamento de seleção:                                                                                                    
      • Exige que crate::objects::selected(app) retorne Some((pos, InlineObject::Image)), o que requer que o caractere de desenho do objeto (OBJ / \u{FFFC}) esteja         
      selecionado como objeto (b.off == a.off + OBJ.len_utf8()).                                                                                                            
      • No clique direito sobre a imagem, o canvas já executa select_at(app, ...) (canvas.rs:1523), garantindo que a imagem esteja devidamente selecionada.                 
      • Se o cursor estiver apenas colapsado adjacente à imagem (por exemplo, ao abrir o menu de contexto via teclado com Shift+F10), crate::widgets::enabled(app, "ui.     
      crop") consulta crop::can_toggle(app), que retorna false. Com isso, o botão no menu fica desabilitado com segurança (ui.add_enabled(false, ...)), sem disparar erros  
      ou alterar a seleção do documento.                                                                                                                                    
  3. Escopo da alteração necessária:                                                                                                                                        
      • A alteração pôde ser estritamente limitada ao item do menu de contexto em canvas.rs. Nenhum bloqueio no motor de corte exigiu modificações adicionais em crop.rs, no
      Ribbon ou na Mini Toolbar.                                                                                                                                            
                                                                                                                                                                            
  ──────                                                                                                                                                                    
  ### 2. Implementação Realizada                                                                                                                                            
                                                                                                                                                                            
  Em canvas.rs:2888-2892:                                                                                                                                                   
                                                                                                                                                                            
  • O rótulo e o comando do item foram alterados de:                                                                                                                        
    item(ui, app, "Size and Crop…", "ui.tab", json!({"tab": "Picture Format"}));                                                                                            
  para:                                                                                                                                                                     
    item(ui, app, "Crop", "ui.crop", json!({}));                                                                                                                            
                                                                                                                                                                            
  • Preservação de traduções e semântica:                                                                                                                                   
      • O rótulo "Crop" já existia previamente no catálogo de tradução em todos os 23 idiomas suportados pelo projeto (por exemplo, em português traduz-se como "Cortar" em 
      crates/ui-egui/src/i18n/pt-br.tsv:291).                                                                                                                               
      • Removeu-se as reticências (…), alinhando a semântica da interface à ação real (ativação direta do modo de corte interativo na tela, e não abertura de diálogo modal).
                                                                                                                                                                            
  ──────                                                                                                                                                                    
  ### 3. Validação e Testes                                                                                                                                                 
                                                                                                                                                                            
  • Testes unitários adicionados em canvas.rs:                                                                                                                              
      1. context_menu_crop_item_activates_crop_mode: Simula o clique com o botão direito na imagem, abre o menu de contexto, clica em "Crop" e valida que                   
      crate::crop::active(h.state()).is_some() (modo de corte ativado com sucesso e alças prontas para desenho).                                                            
      2. crop_is_disabled_when_cursor_is_adjacent_without_selection: Valida o cenário onde o cursor de texto está colapsado imediatamente antes da imagem; comprova que a   
      imagem é detectada contextualmente, mas o comando "ui.crop" permanece desabilitado (enabled == false), sem entrar em modo de corte.                                   
  • Ambiente de compilação:
      • O utilitário cargo não está instalado no PATH do ambiente de execução do container/sandbox, o que impediu a execução de comandos cargo test ou testes visuais       
      automáticos via harness neste momento.
      • Aviso de transparência: Nenhum teste visual em tela interativa foi declarado como executado/aprovado.
  
  ──────
  ### 4. Limitações Restantes
  
  • Imagens rotacionadas: Imagens com rotação diferente de zero (float.rot != 0.0) continuam não suportadas pelo corte interativo na tela, emitindo a mensagem na barra de  
  status definida em crop.rs e mantendo a rotação inalterada.
  • Diálogo modal de tamanho e corte: Não foi implementada uma janela modal para ajuste numérico simultâneo de tamanho e corte (o ajuste numérico continua disponível na    
  Ribbon em Formato da Imagem › Tamanho).