<p align="center">
  <a href="https://getartcraft.com/">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="docs/brand/artcraft-logo-white.svg">
      <img alt="ArtCraft" src="docs/brand/artcraft-logo.svg" width="200">
    </picture>
  </a>
</p>

<h1 align="center">WordCraft</h1>

<p align="center">
  <b>الكتابة وتصميم المستندات؛ إعادة تنفيذ مفتوحة المصدر لبرنامج Microsoft Word وفق منهجية الغرفة النظيفة، مبنية بلغة Rust خالصة.</b>
</p>

<p align="center">
  معالج نصوص سريع ومفتوح المصدر بسير عمل Word الذي تعرفه مسبقاً: الشريط، والأنماط، والجداول، وتعقب التغييرات،
  والمراجع، ودمج المراسلات. يقرأ ملفات .docx ويكتبها، ويعمل أصلياً على macOS وWindows وLinux وBSD،
  وفي المتصفح عبر WebAssembly.<br>
  <i>من فريق ArtCraft.</i>
</p>

<p align="center">
  <img alt="مكتوب بلغة Rust" src="https://img.shields.io/badge/written%20in-Rust-2b47b5?style=flat-square&logo=rust&logoColor=white">
  <img alt="يعمل على macOS وWindows وLinux وBSD والويب" src="https://img.shields.io/badge/runs%20on-macOS%20%C2%B7%20Windows%20%C2%B7%20Linux%20%C2%B7%20BSD%20%C2%B7%20Web-3b5bdb?style=flat-square">
  <img alt="الترخيص: MIT أو Apache-2.0" src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-2b47b5?style=flat-square">
  <img alt="قابل للقيادة من الوكلاء عبر MCP" src="https://img.shields.io/badge/agents-MCP%20%C2%B7%20CLI-3b5bdb?style=flat-square">
</p>

<p align="center">
  <a href="https://discord.gg/artcraft"><img alt="انضم إلى مجتمع ArtCraft على Discord" src="https://img.shields.io/badge/Join%20us%20on%20Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" height="40"></a>
</p>

<p align="center">
  <a href="https://getartcraft.com/apps/wordcraft"><b>WordCraft على getartcraft.com</b></a> ·
  <a href="https://getartcraft.com/">ArtCraft</a> ·
  <a href="https://getartcraft.com/apps">كل Crafting Apps</a>
</p>

<p align="center">
  <a href="README.md">English</a> ·
  <a href="README.ar.md">العربية</a>
</p>

<div dir="rtl">

<br>

<p align="center">
  <img src="docs/images/hero.png" alt="WordCraft مع تبويب «الصفحة الرئيسية» من الشريط مفتوحاً فوق مستند من صفحتين بعنوان The Open Studio Handbook. جزء التنقل على اليسار يسرد عناوين المستند، ومعرض الأنماط يعرض معاينات حية لأنماط عادي وعنوان 1 والعنوان والعنوان الفرعي، وكلمة في الفقرة الأولى محددة" width="100%">
  <br><sub><b>The Open Studio Handbook</b>، المستند النموذجي المدمج في WordCraft: الشريط، ومعرض الأنماط الحي، والمساطر، وجزء التنقل.</sub>
</p>

> [!NOTE]
> **ArtCraft مجتمع من الفنانين من كل المشارب.** رسامون، ومصورون، وصنّاع أفلام، ورسامو توضيح، ومصممون،
> ومحرّكو رسوم، وهواة، ومن التقطوا قلم رصاصهم الأسبوع الماضي. إن كنت تصنع الأشياء فأنت منّا.
> **[تعالَ وسلّم علينا في Discord](https://discord.gg/artcraft).**

<p align="center">
  <a href="#جولة">جولة</a> ·
  <a href="#لماذا-wordcraft">لماذا WordCraft</a> ·
  <a href="#ما-يعمل-اليوم">ما يعمل اليوم</a> ·
  <a href="#البدء-السريع">البدء السريع</a> ·
  <a href="#للوكلاء-cli-وmcp">للوكلاء</a> ·
  <a href="#البنية">البنية</a> ·
  <a href="#خارطة-الطريق">خارطة الطريق</a> ·
  <a href="#التنزيلات">التنزيلات</a> ·
  <a href="#crafting-apps">Crafting Apps</a> ·
  <a href="#الترخيص-والإسنادات">الترخيص والإسنادات</a>
</p>

## جولة

كل لقطة شاشة أدناه هي WordCraft نفسه، مصيَّرة خارج الشاشة بأداة اختبار الواجهة المدمجة فيه
(`cargo run -p wordcraft-ui-egui --example ui_shot`).

<table>
<tr>
<td width="50%" valign="top"><img src="docs/images/review.png" alt="تبويب «المراجعة» مع تشغيل تعقب التغييرات: كلمة forty مدرجة بالأرجواني وكلمة thirty مشطوبة؛ وثلاث عبارات معلَّق عليها مظللة وموصولة بخطوط قائدة متقطعة بفقاعات تعليق في منطقة ترميز رمادية إلى يمين الصفحة" width="100%"><p align="center"><sub><b>المراجعة.</b> تعقب التغييرات، وفقاعات التعليقات في الهامش، والقبول والرفض، والتدقيق الإملائي والنحوي أثناء الكتابة.</sub></p></td>
<td width="50%" valign="top"><img src="docs/images/references.png" alt="تبويب «المراجع» مع صفحتين جنباً إلى جنب: جدول محتويات بخطوط قائدة منقطة وأرقام صفحات في الصفحة الأولى، وجدول منسّق وقائمة مرقمة وارتباط تشعبي في الصفحة الثانية" width="100%"><p align="center"><sub><b>المراجع.</b> جداول المحتويات، والحاشية، والاقتباسات بصيغ APA أو MLA أو Chicago أو IEEE، والفهرس، والتسميات التوضيحية.</sub></p></td>
</tr>
<tr>
<td width="50%" valign="top"><img src="docs/images/design.png" alt="تبويب «التصميم» يعرض معاينات مجموعات الأنماط؛ المستند منسَّق بنسق بخط مذيَّل وعناوين خوخية تسطرها خطوط رفيعة، وخلف النص علامة مائية قطرية باهتة كُتب عليها DRAFT" width="100%"><p align="center"><sub><b>التصميم.</b> الأنساق، ومجموعات الأنماط، وتباعد الفقرات، والعلامات المائية، ولون الصفحة وحدودها.</sub></p></td>
<td width="50%" valign="top"><img src="docs/images/dark.png" alt="WordCraft في الوضع الداكن مع فتح تبويب «إدراج» وإظهار علامات التنسيق: رموز الفقرات في نهايات الفقرات ونقاط مكان المسافات" width="100%"><p align="center"><sub><b>الوضع الداكن</b> مع علامات التنسيق، وتبويب «إدراج»: الجداول، والصور، والأشكال، والارتباطات، والترويسات، والتذييلات، والحقول، والرموز.</sub></p></td>
</tr>
<tr>
<td colspan="2"><img src="docs/images/layout.png" alt="تبويب «تخطيط» بتكبير 80%: الفقرة الأولى تفتتحها ثلاثة أسطر من الحرف الاستهلالي E، ودائرة زرقاء داكنة بجوار المقطع الثاني تلتف حولها فقرته وتصل كلمة art-ist في نهاية السطر، وكل سطر مرقّم في الهامش الأيسر، وحد أزرق رفيع يحيط بالصفحة" width="100%"><p align="center"><sub><b>التخطيط.</b> الحروف الاستهلالية، والتفاف النص حول الصور والأشكال، ووصل الكلمات تلقائياً، وأرقام الأسطر، وحدود الصفحات.</sub></p></td>
</tr>
<tr>
<td colspan="2"><img src="docs/images/backstage.png" alt="صفحة البدء لتبويب «ملف» بتحية صباح الخير، ومصغّرات لقوالب مستند فارغ ودليل الاستوديو وخطاب وسيرة ذاتية وتقرير، وقائمة بالمستندات الأخيرة" width="100%"><p align="center"><sub><b>ملف.</b> ابدأ من قالب، وافتح المستندات الأخيرة، وحرّر الخصائص، وصدّر إلى PDF وسائر الصيغ.</sub></p></td>
</tr>
</table>

## لماذا WordCraft

- **مألوف.** تبويبات الشريط ومجموعاته واختصاراته وسلوكه كما في Word: ‏Enter يُكمل القائمة، وTab يخفضها مستوى،
  وCtrl/⌘+B يجعل الكلمة تحت المؤشر غامقة، ومعرض الأنماط يعاين الأنماط حيّاً، وF4 يعيد، وF7 يدقق الإملاء، وF8 يوسع التحديد.
- **ملفاتك.** يفتح ويحفظ .docx ‏(OOXML)، وكذلك .odt و.rtf و.html و.md و.tex ‏(LaTeX) و.txt؛ ويفتح .doc لبرنامج
  Word 97-2003؛ ويصدّر PDF بنص حقيقي قابل للتحديد، وبارتباطات وإشارات مرجعية.
- **سريع.** تخطيط الفقرات مخزَّن مؤقتاً، فالكتابة في مستند من 188 صفحة تعيد تخطيطه في نحو 1.4 مللي ثانية؛
  والصفحات تُصيَّر عند الطلب.
- **في كل مكان.** قاعدة شيفرة واحدة بلغة Rust لصالح macOS وWindows وLinux وBSD والويب. لا Electron ولا Tauri:‏
  واجهة [egui](https://github.com/emilk/egui) أصلية على وحدة معالجة الرسوميات.
- **مبني للوكلاء.** كل إجراء أمر له معرّف. الأوامر الـ 422 نفسها تقود الشريط، واختصارات لوحة المفاتيح، والبحث في
  الأوامر، وأداة سطر أوامر، وقناة تحكم بصيغة JSON، وخادم MCP.
- **خصوصي.** التدقيق الإملائي والنحوي وكل شيء آخر يعمل دون اتصال.
- **مفتوح.** ‏MIT أو Apache-2.0. غرفة نظيفة: مبني من المواصفات العامة والملاحظة، وكل أصوله أصلية أو مرخّصة بترخيص مفتوح.

## ما يعمل اليوم

| المجال | أبرز ما يعمل |
|---|---|
| **الكتابة** | كتابة سريعة مع محرر أسلوب الإدخال IME، وعلامات تنصيص ذكية، وتصحيح تلقائي، وتنسيق تلقائي للقوائم (`* ` و`1. `)، وشرطات؛ تحديد الكلمة والجملة والفقرة؛ التحديد بالسحب؛ الحافظة مع التنسيق؛ تراجع/إعادة؛ بحث واستبدال بالتعابير النمطية |
| **التنسيق** | الخطوط والأحجام، وأنماط الغامق/المائل/التسطير، والشطب، والرموز السفلية/العلوية، وحالة الأحرف، والتظليل، والألوان، وتباعد المحارف، ونسخ التنسيق، وتغيير الحالة، ومسح التنسيق |
| **الفقرات** | المحاذاة، والإزاحات (قابلة للسحب على المسطرة)، والتباعد، وتباعد الأسطر، وعلامات الجدولة بخطوط قائدة، والحدود، والتظليل، والإبقاء مع التالي، والتحكم في الأسطر المنفردة |
| **الأنماط** | مجموعة أنماط مدمجة، ومعرض حيّ، وجزء الأنماط، وإنشاء/تعديل/تحديث الأنماط، ومجموعات الأنماط، والأنساق |
| **القوائم** | تعداد نقطي، وترقيم، ومستويات متعددة، وإعادة بدء، وتعيين قيمة، وصيغ مخصصة |
| **الجداول** | إدراج بالشبكة، ودمج/تقسيم، وأنماط بصفوف متناوبة، وحدود، وتظليل، وصفوف ترويسة تتكرر عبر الصفحات، وصفوف تنقسم عبر الصفحات، وفرز، وصيغ، ونص ↔ جدول |
| **الصفحات** | الهوامش، والاتجاه، والحجم، والأعمدة، وفواصل الصفحات/الأعمدة/المقاطع، والترويسات والتذييلات (الصفحة الأولى، والفردية/الزوجية)، وأرقام الصفحات، والعلامة المائية، وحدود الصفحات، وأرقام الأسطر، والمحاذاة الرأسية، والحروف الاستهلالية، ووصل الكلمات تلقائياً |
| **الكائنات** | الصور (تحجيم، اقتصاص، تلوين، سطوع/تباين، شفافية، إزالة الخلفية، أنماط صور، تدوير)، والأشكال، ومربعات النص، والوضع العائم مع التفاف النص (مربع، وأعلى وأسفل، وخلف النص أو أمامه) |
| **المراجع** | جدول المحتويات، والحواشي والحاشية الختامية، والاقتباسات وقائمة المراجع (APA وMLA وChicago وIEEE)، والتسميات التوضيحية، وجدول الأشكال، والمراجع المتصالبة، والفهرس، وجدول المراجع القانونية |
| **المراجعة** | تدقيق إملائي ونحوي مع اقتراحات (بالإنجليزية والبولندية، بحسب لغة كل مقطع)، وقاموس المرادفات، وعدد الكلمات، وتعليقات في فقاعات الهامش أو في جزء، وتعقب التغييرات، وقبول/رفض، ومقارنة المستندات، وتقييد التحرير، ومدقق الوصول، وفاحص المستند |
| **المراسلات** | دمج المراسلات من ملفات CSV، وحقول الدمج، وكتلة العنوان، وسطر التحية، والقواعد، والمعاينة، وإنهاء إلى مستند؛ والمظاريف والتسميات |
| **العرض** | تخطيط الطباعة، وتخطيط الويب، والمسودة، ووضع القراءة، والتركيز، والتكبير، وصفحة واحدة/صفحات متعددة، وعرض الصفحة، وجزء التنقل، والمساطر، وخطوط الشبكة، والوضع الداكن؛ والواجهة متاحة بالعربية أو English أو 简体中文 أو 繁體中文 أو 日本語 أو Português (Brasil) أو Español أو Українська أو Српски / Srpski أو Eesti أو Türkçe أو Русский أو Italiano أو Deutsch أو Polski أو Čeština أو Magyar أو Français أو Български (وتتبع لغة النظام افتراضياً) |
| **الملفات** | قراءة/كتابة .docx ‏(يُفتح في Word)، واستيراد .doc لبرنامج Word 97-2003، وتصدير PDF، واستيراد/تصدير .odt و.rtf و.html و.md و.tex و.txt، وصور للصفحات |

الصورة الصادقة، مجالاً مجالاً، في [ROADMAP.md](ROADMAP.md) و[تقييم المطابقة](docs/target-app-parity.md)
و[قائمة الميزات المولَّدة](docs/parity-checklist.md).

## البدء السريع

```sh
git clone https://github.com/storytold/wordcraft
cd wordcraft
cargo run --release -p wordcraft -- --sample        # تطبيق سطح المكتب مع المستند النموذجي
cargo run --release -p wordcraft -- report.docx     # فتح مستند
```

سطر الأوامر:

```sh
wordcraft-cli convert report.docx report.pdf        # docx, pdf, odt, rtf, html, md, tex, txt, png
wordcraft-cli text report.docx                      # نص عادي
wordcraft-cli inspect report.docx                   # البنية بصيغة JSON
wordcraft-cli run --template sample \
  --cmd 'select.text={"text":"Membership"}' --cmd format.bold --save out.docx
```

الويب: ‏`cd apps/wordcraft-web && trunk serve`، ثم افتح <http://127.0.0.1:8771/?sample>.

## للوكلاء: CLI وMCP

صُمم WordCraft ليقوده البشر *ومن* وكلاء الذكاء الاصطناعي على السواء.

```sh
claude mcp add wordcraft -- wordcraft-cli mcp                  # مستندات بلا واجهة
# حصر الوكيل في مجلدات: wordcraft-cli mcp --automation-read-root DIR --automation-write-root DIR
wordcraft --control 7981 &                                     # أو قُد التطبيق قيد التشغيل…
claude mcp add wordcraft-app -- wordcraft-cli mcp --connect 127.0.0.1:7981
```

تتضمن الأدوات ‏`list_commands` و`execute` و`batch` و`type_text` و`select_text` و`inspect_document` و`render_page`
و`save_document`، ومع تطبيق قيد التشغيل أيضاً `screenshot` و`click` و`key`. يستطيع الوكلاء التحقق من عملهم عبر
`inspect_document` دون لقطات شاشة. راجع [docs/mcp.md](docs/mcp.md) و[بروتوكول التحكم](docs/control-protocol.md).
وتسجّل الماكرو أي تسلسل من الأوامر وتعيد تشغيله (`tools.recordMacro` و`tools.macros`).

## السجلات

يكتب تطبيق سطح المكتب سجلاته إلى الخطأ القياسي وإلى `logs/wordcraft.log` بجوار تفضيلاته: على Linux في
`$XDG_CONFIG_HOME/wordcraft/logs/` ‏(افتراضياً `~/.config/wordcraft/logs/`)، وعلى macOS في
`~/Library/Application Support/WordCraft/logs/`، وعلى Windows في `%APPDATA%\WordCraft\logs\`.
التشغيل من قائمة سطح المكتب أو من Dock لا يجاوره طرفية، فهذا الملف هو ما تُرفقه بتقرير الخلل:
الأجزاء غير القابلة للقراءة من ملف .docx، والصور أو الخطوط التي اضطر تصدير PDF إلى تركها، والانهيارات التي
تعافى منها حارس الأوامر، كلها تصل إليه. كل تشغيل ينقل السجل السابق إلى `wordcraft.1.log` ‏(وذاك إلى
`wordcraft.2.log`)، فينجو سجل الجلسة المنهارة من التشغيل التالي؛ ويتوقف نمو السجل عند 16 ميغابايت.
‏`--version` لا يكتب أي ملف، والتشغيلات بـ `WORDCRAFT_NO_PREFS` ‏(تشغيلات اختبار الوكلاء) تسجّل إلى الخطأ
القياسي فقط. نص المستند لا يُسجَّل أبداً؛ ورسالة الانهيار تُسقط النص الذي تقتبسه.

افتراضياً، تسجّل صندوقات WordCraft نفسها بمستوى `info` وكل ما عداها بمستوى `warn`. ويستبدل `RUST_LOG` ذلك
بتوجيهات على طريقة env_logger، مثل `RUST_LOG=debug` أو `RUST_LOG=warn,wordcraft_docx=trace` أو
`RUST_LOG=info,wgpu_core=warn`؛ والتوجيه المنتهي بـ `*` يشمل كل هدف يبدأ به (`wordcraft*=debug`).
المسجِّل في `apps/wordcraft/src/logging.rs`.

## البنية

| الطبقة | الصندوقة | المهمة |
|---|---|---|
| L0 | `wordcraft-geom` | الوحدات والقياسات |
| L1 | `wordcraft-doc` و`wordcraft-fonts` و`wordcraft-proof` | نموذج المستند وتحريره؛ الخطوط والتشكيل؛ التدقيق الإملائي والنحوي ووصل الكلمات |
| L2 | `wordcraft-layout` و`wordcraft-docx` و`wordcraft-formats` | كسر الأسطر، وترقيم الصفحات، والجداول، والملاحظات، واختبار الإصابة؛ ‏OOXML؛ ‏ODT/RTF/HTML/Markdown/TXT |
| L3 | `wordcraft-render` و`wordcraft-pdf` | المُزخرِف النقطي (vello_cpu)؛ ‏PDF ‏(krilla) |
| L4 | `wordcraft-engine` | الجلسة، والتراجع، و422 أمراً، وكتالوج ميزات Word |
| L5 | `wordcraft-mcp` | خادم MCP |
| L6 | `wordcraft-ui-egui` | الواجهة الأمامية على طراز Word ‏(قابلة للتبديل) |
| apps | `wordcraft` و`wordcraft-cli` و`wordcraft-web` | سطح المكتب، وسطر الأوامر، والمتصفح |

ينفّذ `cargo xtask ci` التنسيق، وclippy، ونحو 800 اختبار، وفحص إسناد الأصول، وفحص الطبقية، وبناء wasm.
تعليمات المساهمين والوكلاء في [AGENTS.md](AGENTS.md).

## خارطة الطريق

‏WordCraft في مرحلة **ألفا**. يغطي اليوم 88% من ميزات شريط Word بأوامر فعلية؛ وباحتساب العمق وأمانة الملفات
والاستقرار وما بقي، نقدّر أنه قطع نحو 60% من طريق استبدال Word كله في العمل الجدي، ونحو 55% للعمل الأسبوعي
النموذجي لمهني، ونحو 63% للأساسيات اليومية. تحتاج بيتا إلى اختبار مقابل ملفات .docx من العالم الحقيقي،
والحفاظ على المخططات البيانية وSmartArt عبر دورة كاملة، وترقيم صفحات أقرب إلى Word، وجولة استقرار، وطباعة
أصلية. التفاصيل والتقديرات في [ROADMAP.md](ROADMAP.md) و[مواضع قصورنا](docs/gaps.md).

## التنزيلات

**نزّل WordCraft** من GitHub: يضم [أحدث إصدار](https://github.com/storytold/wordcraft/releases/latest) كل البُنى المذكورة أدناه، وتضم [كل الإصدارات](https://github.com/storytold/wordcraft/releases) الإصدارات السابقة وملاحظاتها. `<ver>` في أسماء الملفات هو رقم الإصدار، ويسرد `SHA256SUMS.txt` بصمة تحقق لكل ملف.

### Windows

| البنية | المُثبِّت | النسخة المحمولة |
|---|---|---|
| x64 ‏(Intel/AMD بمعمارية 64 بت) | `wordcraft-<ver>-windows-x64.msi` | `wordcraft-<ver>-windows-x64-portable.zip` |
| arm64 ‏(Snapdragon وحواسيب ARM الأخرى) | `wordcraft-<ver>-windows-arm64.msi` | `wordcraft-<ver>-windows-arm64-portable.zip` |
| x86 ‏(32 بت) | `wordcraft-<ver>-windows-x86.msi` | `wordcraft-<ver>-windows-x86-portable.zip` |

المثبّتات والملفات التنفيذية موقَّعة رقمياً.

**إذا لم يفتح التطبيق على Windows:** يهيّئ تطبيق سطح المكتب DirectX 12 فقط افتراضياً. السماح لـ wgpu بإنشاء
نسخة OpenGL أيضاً قد يُسقط بعض برامج تشغيل الرسوميات (مثل `atio6axx.dll` من AMD) قبل ظهور النافذة، فيومض
التطبيق في إدارة المهام ثم يخرج. يتجاوز `WGPU_BACKEND` هذا الافتراضي لأغراض التشخيص (مثلاً `dx12` أو
`vulkan`). في PowerShell، من المجلد الحاوي للملف التنفيذي:

```powershell
$env:WGPU_BACKEND = "vulkan"
& .\wordcraft.exe
Remove-Item Env:WGPU_BACKEND                     # استعادة الافتراضي للتشغيلات اللاحقة
```

قد يعيد تجاوز `gl` الصريح سقوط برنامج التشغيل على الأنظمة المتأثرة. أما تفترض الواجهات الخلفية على macOS
وLinux والويب فلا تتغير.

تستخدم بنية **32 بت** OpenGL حتى مع وجود بطاقة رسوميات DirectX 12. ففي Windows ذي 32 بت، يفشل إنشاء خط
أنابيب رسوميات DirectX 12 فوراً ويخرج التطبيق قبل ظهور النافذة. ويظل `WGPU_BACKEND=vulkan` ‏(أو `dx12`)
يتجاوز ذلك.

**إذا خرج التطبيق فور ظهور نافذته مع رسالة "Parent device is lost" في السجل:** يصرّف WordCraft مظلِّلات
DirectX 12 بـ FXC، المصرِّف المدمج في Windows، فلا يحتاج إلى `dxcompiler.dll`/`dxil.dll` ويتجاهل أي نسخة
وضعها برنامج آخر على `PATH`‏ (#603). ويتجاوز `WGPU_DX12_COMPILER` ‏(`fxc` أو `dxc` أو `auto`) هذا؛ ومع
`dxc` يجب أن تجاور `dxcompiler.dll` المتوافقة *و*`dxil.dll` ملف `wordcraft.exe`. كما أن
`WGPU_BACKEND = "vulkan"` يتجنب DirectX 12 كلياً.

**إذا كان التطبيق بطيئاً على حاسوب Windows أقدم:** حين لا يجد DirectX 12 بطاقة رسوميات معها برنامج تشغيل له
(مثلاً رسوميات Intel من الجيل الرابع على Windows 11)، لا يعرض سوى مُصيِّر Microsoft البرمجي الذي يرسم كل
إطار على المعالج. حينها يستخدم WordCraft OpenGL بدلاً منه دون أي ضبط. ويسجل ملف السجل هذا الاختيار (أسطر
تبدأ بـ `graphics:`). ويظل `WGPU_BACKEND` يتجاوزه، مثل `$env:WGPU_BACKEND = "gl"` أو `"dx12"`.

### macOS

| البنية | الملف | ملاحظات |
|---|---|---|
| التطبيق، شامل (Apple silicon + Intel) | `wordcraft-<ver>-macos-universal.dmg` | موقَّع وموثَّق لدى Apple |
| أداة سطر الأوامر، شاملة | `wordcraft-cli-<ver>-macos-universal.zip` | موقَّعة وموثَّقة لدى Apple |

### Linux

| الصيغة | x86_64 | aarch64 ‏(ARM64) | ملاحظات |
|---|---|---|---|
| AppImage | `wordcraft-<ver>-linux-x86_64.AppImage` | `wordcraft-<ver>-linux-aarch64.AppImage` | يعمل في أي مكان؛ يحدّث نفسه عبر [AppImageUpdate](https://github.com/AppImageCommunity/AppImageUpdate) ‏(ملفات `.zsync`) |
| Flatpak | `wordcraft-<ver>-linux-x86_64.flatpak` | `wordcraft-<ver>-linux-aarch64.flatpak` | بيئة معزولة؛ ‏`flatpak install --user <file>` |
| Debian/Ubuntu | `wordcraft-<ver>-linux-x86_64.deb` | `wordcraft-<ver>-linux-aarch64.deb` | |
| Fedora/RHEL/openSUSE | `wordcraft-<ver>-linux-x86_64.rpm` | `wordcraft-<ver>-linux-aarch64.rpm` | |
| حزمة tar | `wordcraft-<ver>-linux-x86_64.tar.gz` | `wordcraft-<ver>-linux-aarch64.tar.gz` | فك الضغط في أي مكان |

‏RISC-V ‏(riscv64): ‏`wordcraft-<ver>-linux-riscv64.tar.gz`، حزمة tar مصرَّفة تشابكياً لصالح glibc 2.39+
‏(Ubuntu 24.04 وDebian 13 وأحدث).

**Wayland:** على سطح مكتب Wayland يعمل التطبيق عبر XWayland متى توفر، لأن winit ‏(مكتبة النوافذ) لا تدعم سحب
الملفات إلى النافذة إلا على X11. ولاستخدام الواجهة الخلفية الأصلية لـ Wayland بدلاً من ذلك (مثلاً لنص أحدّ
مع التحجيم الكسري، على حساب السحب والإفلات للملفات)، شغّله بـ `WORDCRAFT_WAYLAND=1`:

```sh
WORDCRAFT_WAYLAND=1 wordcraft
```

**Arch Linux ‏(AUR).** يحافظ أعضاء من المجتمع على ثلاث حزم في AUR؛ والمشروع نفسه لا ينشر إلى AUR. ثبّت إحداها:
| الحزمة | ما تثبّته | اخترها حين |
|---|---|---|
| [`wordcraft-bin`](https://aur.archlinux.org/packages/wordcraft-bin) | حزمة tar للإصدار أعلاه، مبنية مسبقاً (x86_64 وaarch64) | تريد أحدث إصدار دون ترجمة (الأسرع) |
| [`wordcraft`](https://aur.archlinux.org/packages/wordcraft) | أحدث إصدار، مبني من المصدر بـ `cargo` | تفضّل بناء الإصدارات بنفسك |
| [`wordcraft-git`](https://aur.archlinux.org/packages/wordcraft-git) | فرع `main` الحالي، مبني من المصدر | تريد التغييرات غير الصادرة وتقبل أحياناً بناء معطوباً |
مثلاً `yay -S wordcraft-bin` أو `paru -S wordcraft-bin`.
### FreeBSD

| البنية | الملف |
|---|---|
| x86_64 | `wordcraft-<ver>-freebsd-x86_64.tar.gz` |

### الويب (WebAssembly)

| البنية | الملف | ملاحظات |
|---|---|---|
| موقع ثابت | `wordcraft-web-<ver>.zip` | يعمل في متصفح حديث؛ استضفه على أي خادم ثابت |

## Crafting Apps

‏WordCraft واحد من **Crafting Apps**: أدوات إبداعية مجانية ومفتوحة المصدر من فريق
[ArtCraft](https://getartcraft.com/)، كل واحدة منها مكتوبة بلغة Rust من الصفر، وكل واحدة قادرة على الوقوف وحدها.

| | التطبيق | الغرض | الشيفرة | اعرف المزيد |
|:-:|---|---|---|---|
| <img src="https://raw.githubusercontent.com/storytold/photocraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.photocraft.png" alt="" width="32" height="32"> | **PhotoCraft** | معالجة الصور: طبقات وأقنعة ونصوص وملفات PSD حقيقية | [GitHub](https://github.com/storytold/photocraft) | [الموقع](https://getartcraft.com/apps/photocraft) |
| <img src="https://raw.githubusercontent.com/storytold/vectorcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.vectorcraft.png" alt="" width="32" height="32"> | **VectorCraft** | الرسم المتجه | [GitHub](https://github.com/storytold/vectorcraft) | [الموقع](https://getartcraft.com/apps/vectorcraft) |
| <img src="https://raw.githubusercontent.com/storytold/filmcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.filmcraft.png" alt="" width="32" height="32"> | **FilmCraft** | تحرير الفيديو واللون والصوت | [GitHub](https://github.com/storytold/filmcraft) | [الموقع](https://getartcraft.com/apps/filmcraft) |
| <img src="https://raw.githubusercontent.com/storytold/lightcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.lightcraft.png" alt="" width="32" height="32"> | **LightCraft** | مكتبة الصور وتظهير ملفات raw | [GitHub](https://github.com/storytold/lightcraft) | [الموقع](https://getartcraft.com/apps/lightcraft) |
| <img src="https://raw.githubusercontent.com/storytold/pdfcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.pdfcraft.png" alt="" width="32" height="32"> | **PdfCraft** | قراءة ملفات PDF وتنظيمها وحمايتها | [GitHub](https://github.com/storytold/pdfcraft) | [الموقع](https://getartcraft.com/apps/pdfcraft) |
| <img src="https://raw.githubusercontent.com/storytold/effectcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.effectcraft.png" alt="" width="32" height="32"> | **EffectCraft** | الرسوم المتحركة والتأثيرات البصرية | [GitHub](https://github.com/storytold/effectcraft) | [الموقع](https://getartcraft.com/apps/effectcraft) |
| <img src="https://raw.githubusercontent.com/storytold/designcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.designcraft.png" alt="" width="32" height="32"> | **DesignCraft** | تخطيط الصفحات والنشر | [GitHub](https://github.com/storytold/designcraft) | [الموقع](https://getartcraft.com/apps/designcraft) |
| <img src="https://raw.githubusercontent.com/storytold/wordcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.wordcraft.png" alt="" width="32" height="32"> | **WordCraft** | **الكتابة وتصميم المستندات · أنت هنا** | [GitHub](https://github.com/storytold/wordcraft) | [الموقع](https://getartcraft.com/apps/wordcraft) |

وكذلك [**ArtCraft**](https://getartcraft.com/) نفسه، استوديو الصور والفيديو بالذكاء الاصطناعي لدينا، للفنانين الذين يريدون تحكماً حقيقياً.

<br>

<p align="center">
  <a href="https://discord.gg/artcraft"><img alt="انضم إلى مجتمع ArtCraft على Discord" src="https://img.shields.io/badge/Join%20us%20on%20Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" height="40"></a>
</p>

<h3 align="center">تعال اصنع الأشياء معنا</h3>

<p align="center">
  ‏Discord عندنا هو ملتقى صنّاع الفنون على اختلافهم: من يرسمون، ويصوّرون، وينحتون، ويحرّرون الأفلام،
  وينضدون الحروف، ومن ما زالوا يكتشفون ما يحبون صنعه. شاركنا ما تعمل عليه، واطلب العون،
  وأخبرنا بما تعطّل، أو بما تتمنى أن تفعله هذه الأدوات.
  مهما كان وسيطك ومهما طالت تجربتك، فمكانك بيننا.
</p>

<p align="center">
  <a href="https://discord.gg/artcraft"><b>discord.gg/artcraft</b></a> ·
  <a href="https://getartcraft.com/">getartcraft.com</a> ·
  <a href="https://getartcraft.com/apps">Crafting Apps</a> ·
  <a href="https://getartcraft.com/apps/wordcraft">WordCraft</a>
</p>

## الترخيص والإسنادات

يُرخَّص WordCraft ترخيصاً مزدوجاً بموجب [MIT](LICENSE-MIT) أو [Apache-2.0](LICENSE-APACHE)، أيهما تختار.
حقوق النشر (c) 2026 محفوظة لفريق ArtCraft ومساهمي WordCraft. الإشعارات المطلوبة في [NOTICE](NOTICE).

تحتفظ الخطوط والأيقونات والصور وسائر الأصول المضمّنة بتراخيصها المفتوحة الخاصة بها؛ وكل واحد منها مدرج
باسم مؤلفه ومصدره وترخيصه في [ATTRIBUTION.md](ATTRIBUTION.md).

يأتي قاموس التدقيق الإملائي الإنجليزي وقواعد وصل الكلمات من قائمة كلمات Moby Hyphenator II التي وضعها
Grady Ward وهي في الملك العام. أما التدقيق البولندي فيقرأ قاموس Hunspell ‏`pl_PL` المثبّت على النظام (غير
مضمّن)، ووصل الكلمات البولندي من أنماط TeX لكل من Hanna Kołodziejska وBogusław Jackowski وMarek Ryćko
‏(بخيار MIT). والمستندات والقوالب النموذجية نصوص أصلية كُتبت لصالح WordCraft.

إن اسم ArtCraft وشعاره الكتابي وشعاراته في [`docs/brand/`](docs/brand/) علامات تجارية لفريق
ArtCraft وهي خارج نطاق هذا الترخيص. ولا يجوز استخدامها إلا دون تعديل، وفقط كجزء
من هذا المستودع ومن WordCraft، بموجب [`docs/brand/LICENSE-brand.txt`](docs/brand/LICENSE-brand.txt).
وعلى النسخ المشتقة والنسخ المعدّلة إزالتها.

<sub>إن Microsoft وMicrosoft Word علامتان تجاريتان لمجموعة شركات Microsoft. وإن Adobe وPhotoshop وIllustrator وPremiere Pro وLightroom وAcrobat وAfter Effects وInDesign علامات تجارية أو علامات تجارية مسجلة لشركة Adobe Inc. في الولايات المتحدة و/أو بلدان أخرى. وWordCraft مشروع مستقل مفتوح المصدر لا يتبع شركة Microsoft Corporation ولا Adobe Inc. ولا تتكفّل به أي منهما ولا تدعمه؛ ولا تُستخدم هذه الأسماء إلا لوصف سير العمل المتوافق معه.</sub>

<p align="center">
  <a href="https://getartcraft.com/"><img alt="ArtCraft" src="docs/brand/artcraft-mark.svg" width="28"></a><br>
  <sub>من إنتاج فريق <a href="https://getartcraft.com/">ArtCraft</a> ومجتمعه.</sub>
</p>

</div>
