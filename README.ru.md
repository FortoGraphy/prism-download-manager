🌐 [English](README.md) | [Русский](README.ru.md)

<p align="center">
  <img src="assets/brand/png/app-icon-256.png" width="120" alt="Prism">
</p>

<h1 align="center">Prism</h1>

<p align="center">
  <b>Один луч. Много потоков.</b><br>
  Быстрый, красивый менеджер загрузок с открытым кодом для Windows, macOS и Linux.
</p>

<p align="center">
  <a href="https://github.com/FortoGraphy/prism-download-manager/releases/latest"><img alt="Latest release" src="https://img.shields.io/endpoint?url=https%3A%2F%2Fprismdl.site%2Frelease.json&style=flat-square&cacheSeconds=300"></a>
  <a href="https://github.com/FortoGraphy/prism-download-manager/releases"><img alt="Downloads" src="https://img.shields.io/endpoint?url=https%3A%2F%2Fprismdl.site%2Fdownloads.json&style=flat-square&cacheSeconds=300"></a>
  <a href="https://github.com/FortoGraphy/prism-download-manager/actions/workflows/ci.yml"><img alt="CI" src="https://img.shields.io/endpoint?url=https%3A%2F%2Fprismdl.site%2Ftests.json&style=flat-square&cacheSeconds=300"></a>
  <a href="https://chromewebstore.google.com/detail/prism-download-manager/mmcdeiglkbfbicdjboocmhehodogilfl"><img alt="Chrome Web Store" src="https://img.shields.io/chrome-web-store/v/mmcdeiglkbfbicdjboocmhehodogilfl?style=flat-square&label=Chrome%20Web%20Store&color=4285f4"></a>
  <a href="LICENSE"><img alt="MIT License" src="https://img.shields.io/badge/license-MIT-2ecc71?style=flat-square"></a>
  <img alt="Platforms" src="https://img.shields.io/badge/Windows%20·%20macOS%20·%20Linux-555?style=flat-square">
</p>

<p align="center">
  <a href="https://github.com/FortoGraphy/prism-download-manager/releases/latest"><b>⬇ Скачать</b></a> ·
  <a href="https://prismdl.site/"><b>Сайт</b></a> ·
  <a href="https://youtu.be/IkZu0XN2lLQ"><b>▶ Посмотреть фильм</b></a> ·
  <a href="docs/guide/installation.md">Руководство по установке</a> ·
  <a href="docs/guide/browser-extension.md">Расширение для браузера</a> ·
  <a href="https://ko-fi.com/prismdownloadmanager"><b>☕ Угостить лучом</b></a>
</p>

<p align="center">
  <a href="https://youtu.be/IkZu0XN2lLQ">
    <img src="docs/media/teaser.gif" width="860" alt="Prism в работе: один луч разделяется на много соединений">
  </a>
</p>

---

Prism разделяет каждую загрузку на **до 32 параллельных соединений**, держит все их занятыми
до последнего байта и переживает сбои, сон и вынутый кабель. Нажмите на ссылку в браузере —
Prism перехватит её; наведите курсор на видео и заберите его как **MP4 в нужном качестве — или
только MP3**. Всё это в спокойном интерфейсе из «жидкого стекла» с тёмной и светлой темами.

Бесплатно, с открытым кодом, без рекламы, без аккаунтов, без телеметрии.

## ✨ Почему Prism

| | |
|---|---|
| ⚡ **До 15× быстрее** | На серверах, которые ограничивают каждое соединение, движок Prism с множеством соединений работает до 15× быстрее обычной загрузки из браузера ([замеры](#-замеры)). |
| 🔁 **Никогда не начинает заново** | Пауза, закрытая крышка ноутбука, пропадание электричества — Prism продолжает ровно с того места, где остановился, и проверяет, что файл на сервере не изменился. |
| 🌐 **Живёт в вашем браузере** | Chrome, Edge, Brave, Opera, Vivaldi и Firefox. Загрузки попадают в Prism примерно за четверть секунды. |
| 🎬 **Видео и музыка** | YouTube и [более 1800 сайтов](https://github.com/yt-dlp/yt-dlp/blob/master/supportedsites.md): MP4 до 8K/HDR или только MP3, с субтитрами, главами и SponsorBlock. |
| 🧲 **Любой протокол** | HTTP(S), FTP/FTPS, потоки HLS и DASH, BitTorrent и magnet-ссылки. |
| 🤖 **Модели ИИ** | Скачивайте модели с [Hugging Face](https://huggingface.co) по названию, ссылке или поиску. Prism выбирает нужные файлы (одна версия GGUF, веса в одном формате), скачивает их через множество соединений и сверяет каждый с SHA-256. Работают модели с ограниченным доступом, зеркала вроде hf-mirror.com и папка LM Studio. |
| 🗂 **Порядок** | Категории, очереди с расписанием и ограничением скорости, «выключить по завершении», захват сайта, пакетные загрузки, мониторинг буфера обмена. |
| 🌍 **Ваш язык** | Английский, чешский, немецкий, испанский, французский, итальянский, индонезийский, русский, турецкий, упрощённый китайский и арабский. Prism берёт язык системы, либо выберите свой в «Настройки». |
| 🎨 **Красиво** | Дизайн из «жидкого стекла», тёмная и светлая темы, шесть цветов акцента, мини-окно — и быстро работает с тысячами загрузок. |

<p align="center">
  <img src="docs/screenshots/details-window.png" width="820" alt="Подробности загрузки Prism: 16 живых соединений, карта файла и график скорости">
</p>

## ⬇ Скачать

Возьмите файл для вашей системы в **[последнем релизе](https://github.com/FortoGraphy/prism-download-manager/releases/latest)**:

| Система | Файл |
|---|---|
| **Windows** 10 / 11 | `Prism_<version>_x64-setup.exe` — рекомендуется, права администратора не нужны<br>`…_x64_en-US.msi` для всех пользователей · `…-windows-x64-portable.zip` для флешек |
| **macOS** 11+ Apple Silicon | `Prism_<version>_aarch64.dmg` |
| **macOS** 11+ Intel | `Prism_<version>_x64.dmg` |
| **Ubuntu / Debian / Mint** | `Prism_<version>_amd64.deb` |
| **Fedora / openSUSE** | `Prism-<version>-1.x86_64.rpm` |
| **Любой Linux** | `Prism_<version>_amd64.AppImage` |

> **Первый запуск:** установщики не подписаны сертификтом (сертификаты стоят денег).
> Windows: *Дополнительные сведения → Всё равно выполнить*. macOS: один раз выполните
> `xattr -dr com.apple.quarantine /Applications/Prism.app` (или «Системные настройки →
> Конфиденциальность и безопасность → Всё равно открыть»).
> Пошагово для каждой системы: **[Руководство по установке](docs/guide/installation.md)**.

## 🌐 Расширение для браузера (1 минута)

**[Добавьте Prism из Chrome Web Store](https://chromewebstore.google.com/detail/prism-download-manager/mmcdeiglkbfbicdjboocmhehodogilfl)**
— Chrome, Edge, Brave, Opera, Vivaldi и Helium. Нажмите **Добавьте в Chrome**, закрепите значок
Prism, и он покажет **Подключено**. Магазин держит его в актуальном состоянии.

Prism проводит вас через это и при первом запуске (и в любой момент из «Настройки» →
«Интеграция с браузером»). Замечания для Firefox, Edge/Opera, устранение неполадок и
подробности о конфиденциальности: **[Руководство по расширению для браузера](docs/guide/browser-extension.md)**.

## 📸 Скриншоты

| | |
|---|---|
| ![Download File dialog](docs/screenshots/download-file-dialog.png) | ![Video: MP4 or MP3](docs/screenshots/video-dialog.png) |
| **Щелчок по ссылке** — Prism спрашивает, куда сохранить | **Захват видео** — MP4 + качество, или только MP3 |
| ![Scheduler](docs/screenshots/scheduler.png) | ![Browser integration](docs/screenshots/settings-browser-integration.png) |
| **Очереди и планировщик** — скачивать ночью, затем сон | **Интеграция с браузером** — состояние и починка в один клик |
| ![Download complete](docs/screenshots/complete-dialog.png) | ![Mini window](docs/screenshots/mini-window.png) |
| **Готово** — открыть, показать в папке, проверить | **Мини-окно** — не мешает работать |

## 📊 Замеры

Те же файлы, та же машина, каждый результат проверен по SHA-256
([метод и полные результаты](docs/benchmarks.md)):

| Сценарий | curl (1 соединение) | aria2c | **Prism** |
|---|---|---|---|
| 1 GiB с сервера, который ограничивает каждое соединение | 4,0 MiB/s | 61,0 MiB/s | **60,9 MiB/s** |
| 2 GiB, без ограничения сети | 672 MiB/s | 324 MiB/s | **1029 MiB/s** |
| Сервер разрешает только 4 соединения | 2,0 MiB/s | 8,0 MiB/s | **8,0 MiB/s** (максимум) |
| Сервер, который случайно рвёт соединения | — | ✘ сдался | **37,7 MiB/s** ✔ |
| Реальный интернет, линия ≈ 200 Мбит/с | 25,9 MiB/s | 25,2 MiB/s | **25,1 MiB/s** (скорость линии) |

## 🧠 Как это работает

<p align="center">
  <img src="docs/media/banner.jpg" width="860" alt="Один луч, много потоков">
</p>

* **Динамическая сегментация.** Когда соединение заканчивает свою часть, оно берёт половину
  самой большой оставшейся части — поэтому каждое соединение остаётся занятым до самого конца,
  а «хвост» разбирается на части вместо ожидания одного медленного сокета.
* **Понимает сервер.** Prism замечает, когда сервер отказывается от лишних соединений,
  работает с тем, что он разрешает, и запоминает это для каждого хоста.
* **Сразу на диск.** Данные пишутся прямо в итоговый файл (разреженно на NTFS,
  с предварительным выделением на Linux) — медленного шага «сборки» в конце нет.
* **Устойчиво к сбоям.** Прогресс сохраняется в контрольную точку только после записи данных
  на диск; при продолжении Prism проверяет размер, ETag и Last-Modified через `If-Range`.
* **Rust + Tauri.** Движок на Rust (tokio, rustls), интерфейс на React в нативной оболочке
  Tauri 2 — маленький, быстрый и экономный по памяти.

Подробнее: [ARCHITECTURE.md](ARCHITECTURE.md).

## 🛠 Сборка из исходников

```bash
git clone https://github.com/FortoGraphy/prism-download-manager
cd prism-download-manager
npm ci --prefix app && npm ci --prefix extension
npm run build --prefix extension
cd app && npm run tauri:build
```

Требования и замечания по платформам: [BUILD.md](BUILD.md) (Windows, Linux) ·
[docs/BUILD_MACOS.md](docs/BUILD_MACOS.md) (macOS).

```text
crates/prism-engine       движок загрузок: сегментированный HTTP/FTP, HLS/DASH, продолжение, ограничитель скорости
crates/prism-core         логика приложения: менеджер, SQLite, очереди, планировщик, захват сайта, IPC с браузером
crates/prism-video        управление yt-dlp / ffmpeg, выбор качества, микширование
crates/prism-torrent      BitTorrent / magnet
crates/prism-native-host  мост «браузер ↔ приложение» (native messaging) и регистрация в браузерах
crates/prism-testserver   настраиваемый HTTP-сервер для тестов и замеров
app/                      интерфейс на React + оболочка Tauri 2
extension/                расширение MV3 (Chromium + Firefox)
site/                     prismdl.site
```

## ❓ Частые вопросы

<details><summary><b>Это действительно бесплатно?</b></summary>

Да. Лицензия MIT, без рекламы, без пробного периода, без аккаунта, без телеметрии.
Расширение общается только с приложением на вашем же компьютере.
</details>

<details><summary><b>Windows / macOS говорит, что приложение небезопасно.</b></summary>

Установщики не подписаны платным сертификатом. Весь код здесь, и каждый релиз собирается
GitHub Actions из этого репозитория, с `SHA256SUMS.txt` для проверки вашего файла.
</details>

<details><summary><b>Можно использовать его вместе с IDM или другим менеджером загрузок?</b></summary>

Да, но отключите их расширение для браузера — два расширения, делящие одну загрузку,
работают ненадёжно.
</details>

<details><summary><b>С каких сайтов можно скачивать видео?</b></summary>

С любого сайта, который поддерживает [yt-dlp](https://github.com/yt-dlp/yt-dlp), — Prism
устанавливает его сам и держит в актуальном состоянии (вместе с ffmpeg). Скачивайте только
то, что вы имеете право скачивать.
</details>

<details><summary><b>Как скачать модель с Hugging Face?</b></summary>

Нажмите кнопку модели на панели инструментов (или вставьте название модели вроде
`Qwen/Qwen3-8B`, или её ссылку, в «Добавить URL»). Если у модели несколько версий, выберите
одну и нажмите «Скачать»: файлы попадут в очередь «Hugging Face». Для моделей с ограниченным
доступом и частных моделей добавьте токен доступа в «Настройки» › «Модели ИИ» (Prism также
использует тот, что сохранил `hf auth login`). Токен отправляется только на Hugging Face,
никогда — на серверы, с которых приходят файлы.
</details>

<details><summary><b>Работает ли с браузерами в Flatpak/Snap на Linux?</b></summary>

Браузеры в Flatpak не могут общаться с приложениями на рабочем столе (ограничение песочницы).
Используйте версию браузера в deb/rpm для полной интеграции.
</details>

## 🤝 Участие в проекте

Отчёты об ошибках, идеи и pull request очень приветствуются — см. [CONTRIBUTING.md](CONTRIBUTING.md).
Если Prism экономит вам время, **⭐ этому репозиторию помогает большему числу людей узнать о нём.**

<a href="https://star-history.com/#FortoGraphy/prism-download-manager&Date">
  <img src="https://api.star-history.com/svg?repos=FortoGraphy/prism-download-manager&type=Date" width="600" alt="Star history">
</a>

## 📄 Лицензия

[MIT](LICENSE) для собственного кода Prism. Встроенные и скачиваемые компоненты сохраняют
свои лицензии — см. [THIRD_PARTY.md](THIRD_PARTY.md).

<sub>Prism — независимый проект. Он не связан с Internet Download Manager или Tonec Inc.,
не одобрён ими и не основан на них; все названия — товарные знаки их владельцев.</sub>
