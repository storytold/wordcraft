# Investigação Detalhada: `ui.crop` e Ausência das Alças de Corte

Este documento apresenta a análise técnica sobre o comando `ui.crop`, o motivo pelo qual ele não está sendo acionado e não exibe as alças de corte interativas na imagem, e os possíveis caminhos de solução.

---

## 1. Como o `ui.crop` Funciona na Arquitetura

O sistema de corte de imagens no WordCraft foi projetado para funcionar no canvas como um modo interativo (semelhante ao Microsoft Word):

1. **Estado do Canvas:**
   - O estado do modo de corte é controlado por `app.canvas.crop: Option<Pos>` ([`crates/ui-egui/src/canvas.rs:108`](../crates/ui-egui/src/canvas.rs#L108)).
   - Quando ativado, guarda a posição `Pos` do caractere de desenho `InlineObject::Image` no documento.

2. **Acionamento (`ui.crop`):**
   - O comando de UI `"ui.crop"` é registrado em [`crates/ui-egui/src/lib.rs:1532`](../crates/ui-egui/src/lib.rs#L1532) e delega para `crop::toggle(app)` em [`crates/ui-egui/src/crop.rs:178`](../crates/ui-egui/src/crop.rs#L178).
   - Se já estiver ativo (`active(app).is_some()`), desativa (`crop: false`).
   - Se inativo, valida a seleção da imagem e define `app.canvas.crop = Some(pos)`.

3. **Renderização Visual das Alças (`crop::paint`):**
   - No loop de desenho do canvas, [`crates/ui-egui/src/objects.rs:500`](../crates/ui-egui/src/objects.rs#L500) chama `crate::crop::paint(...)`.
   - Se `crop::paint` retornar `true`, ele intercepta o desenho da moldura tradicional de redimensionamento e desenha:
     - As regiões cortadas escurecidas (`t.crop_dim`) ao redor da imagem ([`crop.rs:347`](../crates/ui-egui/src/crop.rs#L347)).
     - O retângulo interno e externo da área visível.
     - **8 alças pretas de corte** (`paint_handles` em [`crop.rs:357`](../crates/ui-egui/src/crop.rs#L357)): 4 cantos em formato de "L" espesso e 4 barras centrais nas bordas superior, inferior, esquerda e direita.

4. **Interação com Mouse (`crop::pointer`):**
   - Identifica se o clique inicial atingiu uma das 8 alças (`frame::handle_at`).
   - Inicia o estado de arrasto `app.canvas.crop_drag: Option<CropDrag>`.
   - Ao soltar o botão do mouse, despacha o comando do engine `"picture.crop"` ([`crates/engine/src/cmd/objects.rs:961`](../crates/engine/src/cmd/objects.rs#L961)) com os novos valores fracionários de corte (`left`, `top`, `right`, `bottom`) e nova largura/altura mantendo a escala.

---

## 2. Por que o `ui.crop` Não Está Acionando / Alças Não Aparecem?

A investigação revelou múltiplos fatores e cenários que causam ou contribuem para essa falha:

### 2.1. Causa Principal no Menu de Contexto da Imagem
- **O item de menu sequer chama `ui.crop`:**
  No arquivo [`crates/ui-egui/src/canvas.rs:2891`](../crates/ui-egui/src/canvas.rs#L2891):
  ```rust
  if picture {
      item(ui, app, "Change Picture…", "ui.changePicture", json!({}));
      item(ui, app, "Reset Picture", "picture.reset", json!({}));
      item(ui, app, "Size and Crop…", "ui.tab", json!({"tab": "Picture Format"}));
  }
  ```
  O item traduzido como `"Tamanho e corte…"` executa `"ui.tab"` com `{"tab": "Picture Format"}`. Ele **não executa `"ui.crop"`**.
  - O clique apenas define a aba da Ribbon como `"Picture Format"`.
  - Como a imagem já estava clicada/selecionada, a aba `"Picture Format"` geralmente já era a aba visível.
  - O usuário clica em "Tamanho e corte…" esperando que as alças de corte apareçam na imagem, mas o código nunca solicita a ativação do modo `app.canvas.crop`.

---

### 2.2. Divergência na Detecção de Imagem Selecionada (Caret Adjacente vs Seleção de Objeto)
Há uma discrepância sutil entre o que o Ribbon considera como "imagem selecionada" e o que o `crop.rs` exige:

- **Na Ribbon ([`crates/ui-egui/src/ribbon.rs:15`](../crates/ui-egui/src/ribbon.rs#L15)):**
  ```rust
  pub fn has_picture_selected(s: &wordcraft_engine::Session) -> bool {
      matches!(wordcraft_engine::cmd::objects::selected(s), Some((_, wordcraft_doc::para::InlineObject::Image { .. })))
  }
  ```
  `cmd::objects::selected` considera uma imagem ativa **mesmo se a seleção for um cursor colapsado encostado na imagem** (`a == b`).
- **No `crop.rs` ([`crates/ui-egui/src/crop.rs:157-164`](../crates/ui-egui/src/crop.rs#L157-L164)):**
  ```rust
  fn picture(app: &WordApp) -> Option<Picture> {
      match crate::objects::selected(app)? { ... }
  }
  ```
  `crate::objects::selected(app)` invoca `object_selection(&app.session)`, que exige:
  ```rust
  b.off == a.off + wordcraft_doc::para::OBJ.len_utf8()
  ```
  Ou seja, **exige que a imagem esteja estritamente selecionada como objeto (o caractere `\u{FFFC}` em destaque)**.
- **Consequência:**
  Se o usuário estiver com o cursor de texto posicionado imediatamente antes ou depois da imagem, a Ribbon exibirá a guia *"Formato da imagem"* e o grupo *"Size"*, mas o botão *"Crop"* na Ribbon estará desabilitado (`widgets::enabled` retorna `false`) ou, se for chamado via comando, falhará com o erro `"select a picture first"`.

---

### 2.3. Bloqueio Completo em Imagens com Rotação (`float.rot != 0.0`)
Em [`crates/ui-egui/src/crop.rs:158-164`](../crates/ui-egui/src/crop.rs#L158-L164):
```rust
fn picture(app: &WordApp) -> Option<Picture> {
    match crate::objects::selected(app)? {
        (pos, InlineObject::Image { w, h, crop, media, float, .. }) if wordcraft_geom::normalize_degrees(float.rot) == 0.0 => {
            Some(Picture { pos, w: *w, h: *h, crop: *crop, media: media.clone(), flip_h: float.flip_h, flip_v: float.flip_v })
        }
        _ => None,
    }
}
```
- Se a imagem tiver qualquer ângulo de rotação (mesmo fracionário, ou 90°/180°), `picture(app)` retorna `None`.
- `crop::toggle` aborta retornando `Err` com a mensagem:
  *"Crop on the page works on unturned pictures: set Rotation to 0° first, or use the crop boxes"*.
- Nenhuma alça é desenhada em imagens rotacionadas.

---

### 2.4. Descarte Imediato por Clique Fora da Imagem (`leave(app)`)
Em [`crates/ui-egui/src/crop.rs:261-267`](../crates/ui-egui/src/crop.rs#L261-L267):
```rust
if f.contains(at) {
    return Some(true); // inside the picture: crop mode stays on
}
// Outside: crop mode ends with the crop kept, and the press goes on as usual.
leave(app);
None
```
- No momento em que qualquer clique primário do mouse ocorre no canvas fora do retângulo `f` da imagem, `leave(app)` é chamado imediatamente, limpando `app.canvas.crop = None`.
- Se a imagem perder o foco ou se houver um clique acidental ligeiramente fora da borda (por exemplo, ao tentar clicar em uma alça externa que não caia perfeitamente dentro do limite), o modo de corte é cancelado instantaneamente.

---

### 2.5. Falta de Atribuição de Foco ao Canvas ao Acionar via Ribbon
Em [`crates/ui-egui/src/widgets.rs:126-131`](../crates/ui-egui/src/widgets.rs#L126-L131):
```rust
pub(crate) fn run_from_ribbon(app: &mut WordApp, id: &str, params: Value) {
    let _ = app.run(id, params);
    if !id.starts_with("ui.") {
        app.canvas.want_focus = true;
    }
}
```
- Como `id` é `"ui.crop"`, ele começa com `"ui."`. Portanto, `app.canvas.want_focus = true` **não** é executado ao clicar no botão "Crop" da Ribbon.
- Sem o foco do teclado no canvas, pressionar `Escape` ou `Enter` para concluir o corte não é processado pelo `crop::key` até que o usuário clique novamente no canvas.

---

## 3. Possíveis Soluções e Correções

Para resolver a questão e permitir que o usuário corte imagens com facilidade e veja as alças de corte, as seguintes soluções podem ser aplicadas:

### Opção 1: Corrigir o Menu de Contexto para Acionar `ui.crop` Diretamente
Modificar o menu de contexto da imagem em [`crates/ui-egui/src/canvas.rs:2888-2892`](../crates/ui-egui/src/canvas.rs#L2888-L2892):
- Substituir a chamada `"ui.tab"` por `"ui.crop"`:
  ```rust
  if picture {
      item(ui, app, "Change Picture…", "ui.changePicture", json!({}));
      item(ui, app, "Reset Picture", "picture.reset", json!({}));
      item(ui, app, "Crop", "ui.crop", json!({}));
  }
  ```
- **Resultado:** Ao clicar com o botão direito na imagem e selecionar "Crop" (ou "Cortar"), o comando `ui.crop` é executado, ativando imediatamente as 8 alças pretas de corte e o escurecimento ao redor da imagem na tela.

---

### Opção 2: Separar em "Cortar" (`ui.crop`) e "Tamanho e Posição…" (Diálogo Modal)
Seguindo o padrão do Microsoft Word:
1. No menu de contexto:
   - Adicionar o item **"Cortar"** (*Crop*), sem reticências, chamando `"ui.crop"`.
   - Manter ou criar o item **"Tamanho e Posição…"** (*Size and Position…*), com reticências, apontando para uma caixa de diálogo real (`ui.dialog`, ex: `name: "picture_size"`).
2. Criar a caixa de diálogo modal de Tamanho e Posição:
   - Campos para Largura e Altura numéricas.
   - Bloqueio de proporção (*Lock Aspect Ratio*).
   - Sliders ou campos numéricos para Corte (Esquerda, Direita, Superior, Inferior).
   - Opções de quebra de texto (*Wrap Text*).

---

### Opção 3: Auto-seleção do Objeto ao Acionar `ui.crop`
Em [`crates/ui-egui/src/crop.rs:183`](../crates/ui-egui/src/crop.rs#L183):
- Se o usuário acionar `ui.crop` quando o cursor estiver adjacente à imagem (situação detectada por `cmd::objects::selected`), em vez de retornar erro `"select a picture first"`, o código pode automaticamente expandir a seleção para englobar o objeto de imagem (`app.session.sel = Selection { anchor: pos, focus: pos + OBJ }`).
- Isso evita que o botão apareça desabilitado na Ribbon quando o usuário simplesmente clicou ao lado da imagem.

---

### Opção 4: Garantir Foco no Canvas após Ativar o Corte
No manipulador de `"ui.crop"` em `crates/ui-egui/src/lib.rs` ou `crop::toggle`:
- Adicionar `app.canvas.want_focus = true;` ao ativar o modo de corte.
- Isso assegura que o canvas receba imediatamente eventos de teclado, permitindo que `Escape` ou `Enter` funcionem de primeira para confirmar ou cancelar o corte.

---

### Opção 5: Suporte Futuro a Imagens Rotacionadas
Para permitir corte em imagens com rotação diferente de 0°:
- Permitir que `picture(app)` retorne a imagem mesmo com `float.rot != 0.0`.
- Aplicar a rotação nas coordenadas dos cantos das alças em `paint_handles` e em `frame::handle_at` (utilizando as funções de rotação já existentes em `wordcraft_geom::normalize_degrees` e no módulo `frame`).
