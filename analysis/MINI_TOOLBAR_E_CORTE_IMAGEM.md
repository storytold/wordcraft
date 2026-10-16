# Análise: Pop-up de Seleção de Texto e Opção "Tamanho e Corte" de Imagem

Este documento registra a localização no código-fonte, o funcionamento e o diagnóstico de dois componentes da interface do **WordCraft**:
1. O pop-up flutuante que surge ao selecionar texto (**Mini Toolbar**).
2. O item de menu de contexto **"Tamanho e corte…"** (*Size and Crop…*) ao clicar com o botão direito em uma imagem, que atualmente parece não fazer nada.

---

## 1. Pop-up ao Selecionar Texto (Mini Toolbar)

No WordCraft (espelhando o comportamento do Microsoft Word), ao selecionar um trecho de texto, surge uma barra flutuante semitransparente com as ações de formatação mais frequentes.

### 1.1. Arquivo Principal
- **[`crates/ui-egui/src/mini_toolbar.rs`](crates/ui-egui/src/mini_toolbar.rs)**

### 1.2. Estrutura e Controles
O pop-up é desenhado em duas linhas (`mini_toolbar::content`):
- **Linha 1:**
  - Seletor de fonte (`mini_font` via `widgets::font_combo`)
  - Tamanho da fonte (`mini_size` via `widgets::combo`)
  - Aumentar tamanho da fonte (`format.growFont`)
  - Diminuir tamanho da fonte (`format.shrinkFont`)
- **Linha 2:**
  - Negrito (`format.bold`)
  - Itálico (`format.italic`)
  - Sublinhado (`format.underline`)
  - Tachado (`format.strikethrough`)
  - Separador vertical
  - Cor de realce do texto / Marca-texto (`format.highlight` com menu dropdown)
  - Cor da fonte (`format.color` com menu dropdown e seletor)
  - Separador vertical
  - Alinhamentos de parágrafo: Esquerda (`para.alignLeft`), Centralizado (`para.alignCenter`), Direita (`para.alignRight`), Justificado (`para.justify`)

### 1.3. Onde é Invocado e Renderizado
- **Cálculo da âncora visual:**
  - [`crates/ui-egui/src/canvas.rs:1296`](crates/ui-egui/src/canvas.rs#L1296)
  - `app.canvas.mini_anchor = crate::mini_toolbar::visible_anchor(screen_rects, visible_clip);`
- **Renderização e descarte no loop do canvas:**
  - [`crates/ui-egui/src/canvas.rs:1579-1584`](crates/ui-egui/src/canvas.rs#L1579-L1584)
  ```rust
  crate::mini_toolbar::track_dismissal(app, ui.ctx(), area);
  if let Some(sel) = app.canvas.mini_anchor
      && app.ui.mini_toolbar
  {
      crate::mini_toolbar::show(app, ui.ctx(), sel, out.inner_rect);
  }
  ```
- **Camada egui:**
  - Renderizado na camada `Order::Middle` (`mini_toolbar.rs:184`), ficando acima da página do documento, porém abaixo de menus de contexto, dropdowns do ribbon e caixas de diálogo modais.

### 1.4. Regras de Visibilidade (`mini_toolbar::visible`)
O pop-up só aparece se todas as seguintes condições forem verdadeiras:
1. `!app.session.view.read_mode`: Desativado no modo leitura.
2. `app.dialog.is_none() && app.colors.is_none() && !app.ui.backstage`: Não aparece durante diálogos modais ou tela Backstage.
3. `!app.canvas.context_menu_open`: Suprimido enquanto o menu de contexto do botão direito estiver aberto.
4. `!app.canvas.drag_selecting() && app.canvas.table_drag.is_none()`: Não aparece enquanto o mouse ainda estiver arrastando a seleção (só aparece no *mouse-up*).
5. `app.ui.keytips == Phase::Off`: Não aparece durante navegação por atalhos de teclado (KeyTips).
6. `app.session.cell_range().is_none()`: Não aparece se células inteiras de tabela estiverem selecionadas.
7. `app.canvas.mini_dismissed.as_ref() != Some(&app.session.sel)`: Se o usuário clicar fora do documento para focar no ribbon, ele fecha e não reabre para a mesma seleção até nova seleção.
8. `!app.session.sel.is_collapsed()`: Requer seleção ativa (não aparece com apenas o cursor piscando).

### 1.5. Configuração do Usuário
- Em **Opções / Preferências**:
  - [`crates/ui-egui/src/options.rs:184`](crates/ui-egui/src/options.rs#L184)
  - Campo: `app.ui.mini_toolbar` ("*Show the mini toolbar when text is selected*")
  - Definido em [`crates/ui-egui/src/lib.rs:313`](crates/ui-egui/src/lib.rs#L313), padrão `true`.

---

## 2. Opção "Tamanho e corte…" no Menu de Contexto da Imagem

### 2.1. Localização no Código
- **Texto original (Inglês):** `"Size and Crop…"`
- **Tradução (pt-br):** `"Tamanho e corte…"` em [`crates/ui-egui/src/i18n/pt-br.tsv:1405`](crates/ui-egui/src/i18n/pt-br.tsv#L1405).
- **Construção do menu de contexto:**
  - [`crates/ui-egui/src/canvas.rs:2888-2892`](crates/ui-egui/src/canvas.rs#L2888-L2892)
  ```rust
  if picture {
      item(ui, app, "Change Picture…", "ui.changePicture", json!({}));
      item(ui, app, "Reset Picture", "picture.reset", json!({}));
      item(ui, app, "Size and Crop…", "ui.tab", json!({"tab": "Picture Format"}));
  }
  ```

### 2.2. Por que Atualmente "Não Faz Nada"?
1. **Comando acionado é apenas troca de aba:**
   O item executa o comando `"ui.tab"` passando `{"tab": "Picture Format"}` ([`crates/ui-egui/src/lib.rs:1283-1292`](crates/ui-egui/src/lib.rs#L1283-L1292)):
   ```rust
   "ui.tab" => {
       let tab = s("tab").unwrap_or("Home").to_string();
       if tab == "File" {
           self.ui.backstage = true;
       } else {
           self.ui.tab = tab;
           self.ui.backstage = false;
       }
       json!({"tab": self.ui.tab})
   }
   ```
2. **A aba já costuma estar ativa:**
   Quando uma imagem é selecionada ou clicada com o botão direito, a aba contextual `"Picture Format"` já é adicionada à barra de guias ([`crates/ui-egui/src/ribbon.rs:110-112`](crates/ui-egui/src/ribbon.rs#L110-L112)). Se a aba ativa já for `"Picture Format"`, executar `"ui.tab"` com esse valor redefine a mesma string e não gera nenhuma alteração visual na tela.
3. **Falsa indicação de diálogo:**
   A string possui reticências (`…`), o que na convenção de interfaces (e do Word) indica a abertura de uma caixa de diálogo ou janela modal. No entanto, nenhum diálogo é aberto.
4. **Não ativa o modo de corte da tela:**
   O WordCraft possui suporte completo a corte interativo via mouse na tela através do comando `"ui.crop"` ([`crates/ui-egui/src/crop.rs:177`](crates/ui-egui/src/crop.rs#L177)), mas o menu de contexto não chama esse comando.
5. **No Microsoft Word original:**
   No Word, a opção equivalente no menu de clique direito da imagem é **"Tamanho e Posição…"** (*Size and Position…*), que abre a caixa de diálogo de Layout (com abas de Posição, Quebra de Texto e Tamanho/Corte). No WordCraft atual, essa caixa de diálogo modal de Tamanho ainda não existe; os controles numéricos de tamanho e corte existem exclusivamente dentro da faixa de opções (Ribbon) na aba `Picture Format › Size` ([`crates/ui-egui/src/ribbon.rs:2368-2416`](crates/ui-egui/src/ribbon.rs#L2368-L2416)).

### 2.3. Possíveis Direcionamentos para Implementação Futura
- **Opção A (Ativar ferramenta de corte):**
  Trocar ou dividir a opção para disparar `"ui.crop"`, ativando imediatamente as alças pretas de corte na imagem selecionada.
- **Opção B (Criar o diálogo modal de Tamanho e Posição / Corte):**
  Implementar um diálogo em `crates/ui-egui/src/dialogs_*.rs` (ex: `ui.dialog` com `name: "picture_size"`), permitindo ajustar largura, altura, proporção e recortes (E/D/C/B) numericamente, correspondendo fielmente ao Word.
