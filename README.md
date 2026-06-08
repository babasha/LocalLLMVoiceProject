# Voice Translator

Real-time голосовой переводчик с полностью локальной обработкой. Говоришь по-русски — из колонки звучит английский. Всё работает на твоём GPU, без интернета и без облачных API.

```
Микрофон → VAD → STT → LLM-перевод → [TTS] → Консоль/Колонка
```

## Быстрый старт

```bash
git clone https://github.com/<your-repo>/LocalLLMVoiceProject.git
cd LocalLLMVoiceProject
./setup.sh
./run.sh
```

`setup.sh` сам проверит зависимости, скачает все модели (~3.1 GB) и соберёт проект.

## Запуск на Windows (нативно, GPU)

`setup.sh`/`run.sh` рассчитаны на Linux. На Windows проект собирается и работает **нативно** (проверено на RTX 5070 Laptop, CUDA 13.3) через batch-обёртки в корне репозитория — WSL не нужен.

### Разовая установка тулчейна (через `winget`)

- **Rust** (MSVC): `Rustlang.Rustup`
- **VS Build Tools** (C++): `Microsoft.VisualStudio.2022.BuildTools` + workload `Microsoft.VisualStudio.Workload.VCTools`
- **CMake**: `Kitware.CMake`
- **CUDA Toolkit** 12.8+ (для Blackwell/RTX 50xx — 13.x): `Nvidia.CUDA`
- **LLVM** (libclang для bindgen): `LLVM.LLVM`

### Сборка и запуск

```bat
build_win.bat   :: сборка (MSVC + CUDA, генератор Ninja); артефакты в C:\cargo-target (вне OneDrive)
run_win.bat     :: полный пайплайн в консоли (mic → STT → перевод)
ui.bat          :: небольшое окно с живым выводом RU → EN + история фраз
```

### Важные нюансы Windows

- **Модели — вне OneDrive.** Лежат в `C:\voice-translator\models`, пути в `config/default.toml` абсолютные. В синхронизируемой папке OneDrive файлы становятся placeholder'ами, и llama.cpp не может прочитать GGUF (`failed to read magic`).
- **Нативные библиотеки sherpa-onnx** для Windows — в `libs/sherpa-onnx-win` (взяты из релиза k2-fsa `win-x64-shared-MD-Release`; `onnxruntime.lib` сгенерирована из DLL). Эти DLL копируются рядом с exe, иначе грузится системная `onnxruntime.dll` (Windows ML, версия старее) из `System32`.
- **CUDA 13**: рантайм-DLL (`cublas64_13`, `cudart64_13`) лежат в `bin\x64` (а не `bin`); cmake собирает llama.cpp генератором **Ninja** (генератор Visual Studio спотыкается на `CudaToolkitDir`).
- **`llama-cpp-2 = 0.1.146`** — требуется для архитектуры `qwen35` (Qwen3.5: Gated DeltaNet + MoE); более старые версии модель не грузят.
- **Аудио** захватывается в нативном формате устройства (напр. 48 кГц стерео) и ресемплится в 16 кГц моно «на лету» — жёсткий запрос 16 кГц на многих микрофонах не поддерживается.

## Требования

| Что | Минимум | Рекомендуется |
|-----|---------|---------------|
| OS | Linux (Ubuntu 22.04+) | Ubuntu 24.04 |
| GPU | NVIDIA с 4+ GB VRAM | 6+ GB VRAM |
| CUDA Toolkit | 12.2+ | 12.6+ |
| RAM | 8 GB | 16 GB |
| Rust | 1.75+ | latest stable |
| CMake | 3.25+ | 3.28+ |

```bash
# Ubuntu — установка системных зависимостей
sudo apt install build-essential cmake libasound2-dev pkg-config

# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Режимы работы

```bash
# Полный пайплайн: микрофон → распознавание → перевод → вывод
./run.sh

# Только распознавание речи (без перевода)
LD_LIBRARY_PATH=./libs/sherpa-onnx:$LD_LIBRARY_PATH \
  ./target/release/voice-translator --mode stt

# Проброс звука (тест микрофона/колонки)
LD_LIBRARY_PATH=./libs/sherpa-onnx:$LD_LIBRARY_PATH \
  ./target/release/voice-translator --mode passthrough

# Посмотреть аудиоустройства
./target/release/voice-translator --list-devices

# Переопределить языки
./run.sh --source-lang en --target-lang ru
```

## Архитектура

### Общая схема

```
┌──────────┐    ┌─────────┐    ┌───────────┐    ┌────────────┐    ┌─────────┐
│ Микрофон │───→│   VAD   │───→│    STT    │───→│  Перевод   │───→│ Консоль │
│  (cpal)  │    │(Silero) │    │ (GigaAM)  │    │  (Qwen3.5) │    │  [TTS]  │
└──────────┘    └─────────┘    └───────────┘    └────────────┘    └─────────┘
   16kHz          512 сэмплов     ~300 MB          ~2.7 GB
   mono           за фрейм       ONNX INT8        GGUF Q4
```

### Потоки и синхронизация

Приложение использует **3 потока** в full-режиме:

```
Поток 1: Audio (cpal)              Поток 2: STT                 Поток 3: Перевод
─────────────────────              ──────────────                ─────────────────
cpal callback (real-time)          Читает из ring buffer         Получает текст из канала
  │                                  │                              │
  ├─ push() в ring buffer ──────→    ├─ VAD (512 сэмплов)          ├─ Drain до последнего
  │  (lock-free, zero-alloc)         │                              │  (пропускает устаревшие)
  │                                  ├─ STT (GigaAM)               │
  │                                  │                              ├─ LLM translate
  │                                  └─ send(SttEvent) ──────────→  │
  │                                     через crossbeam             └─ Обновляет дисплей
```

**Почему так:**

- **Ring buffer (`rtrb`)** — lock-free и wait-free. Аудио-callback работает в реальном времени (ALSA/WASAPI), ему нельзя блокироваться или выделять память. Обычный `Mutex` или `mpsc::channel` тут создали бы джиттер.

- **Crossbeam channel** (bounded, ёмкость 16) — между STT и переводом. Перевод медленнее распознавания, поэтому канал может наполниться. Решение: "drain to latest" — translation-поток при получении сообщения тут же вычитывает все оставшиеся и берёт только последнее. Так переводится актуальный текст, а не устаревший.

- **AtomicBool для shutdown** — все потоки проверяют `is_shutdown()` без блокировок. Ctrl+C ставит флаг, потоки завершаются на следующей итерации.

- **Нет async/await** — STT и LLM это CPU-bound задачи. Async тут не даёт преимуществ, только усложняет код.

## Компоненты

### VAD — Voice Activity Detection

**Модель:** [Silero VAD v5](https://github.com/snakers4/silero-vad) (~630 KB, ONNX)
**Обёртка:** sherpa-onnx

Определяет, когда человек говорит, а когда тишина. Без VAD пришлось бы отправлять на распознавание непрерывный поток аудио, что:
- Тратит GPU впустую на тишину
- Не даёт понять где заканчивается фраза
- Создаёт бессмысленные "транскрипции" шума

**Как работает:**
1. Получает аудио по 512 сэмплов (~32 мс при 16kHz)
2. Возвращает вероятность речи (0.0 — 1.0)
3. Если вероятность > порога (0.5) — речь
4. Когда тишина длится > 1500 мс — сегмент закончен
5. `pop_segment()` возвращает весь аудиофрагмент речи для STT

**Почему Silero:** маленькая (630 KB), быстрая, точная, работает через ONNX без дополнительных зависимостей.

```
config/default.toml:
[vad]
threshold = 0.5              # Порог детекции речи
silence_duration_ms = 1500   # Тишина до конца сегмента
min_speech_duration_ms = 300  # Минимум чтобы не ловить щелчки
max_speech_duration_ms = 15000 # Максимум (принудительный flush)
```

### STT — Speech-to-Text

**Модель:** [GigaAM-v3](https://github.com/salute-developers/GigaAM) от Сбера (~320 MB, ONNX INT8)
**Обёртка:** sherpa-onnx (OfflineRecognizer)
**Источник:** [Smirnov75/GigaAM-v3-sherpa-onnx](https://huggingface.co/Smirnov75/GigaAM-v3-sherpa-onnx)

Преобразует аудио в текст. Это end-to-end CTC-модель: принимает сырое аудио, возвращает текст с пунктуацией.

**Почему GigaAM, а не Whisper:**
- Заточена под русский язык — качество распознавания русской речи выше
- INT8-квантизация — быстрее на GPU при минимальной потере качества
- Встроенная пунктуация — не нужен отдельный пост-процессинг
- Через sherpa-onnx работает offline без Python

**Почему INT8:** полная модель весит 886 MB, INT8 — 320 MB. Разница в качестве минимальна, а скорость инференса выше.

```
config/default.toml:
[stt]
model_path = "models/gigaam_v3_e2e_ctc_int8.onnx"
tokens_path = "models/gigaam_v3_e2e_ctc_tokens.txt"
```

### Перевод — LLM Translation

**Модель:** [Qwen3.5-4B](https://huggingface.co/unsloth/Qwen3.5-4B-GGUF) (2.7 GB, GGUF Q4_K_M)
**Обёртка:** llama-cpp-2 `0.1.146` (Rust bindings для llama.cpp) с CUDA — версия важна: архитектура `qwen35` поддерживается только в свежем llama.cpp

Переводит распознанный текст с одного языка на другой используя LLM.

**Почему LLM, а не специализированный переводчик:**
- Понимает контекст и идиомы лучше, чем seq2seq-модели
- Одна модель для любых языковых пар — не нужно скачивать отдельные модели для каждого направления
- Qwen3.5 хорошо работает с мультиязычными задачами

**Почему Qwen3.5-4B:**
- 4B параметров — достаточно для качественного перевода коротких фраз
- Влезает в 4-6 GB VRAM в квантизации Q4_K_M
- Qwen3.5 — одна из лучших моделей такого размера для мультиязычных задач

**Почему Q4_K_M квантизация:**
- Баланс между качеством и размером/скоростью
- Q4_K_M сохраняет ~95% качества оригинальной модели
- 2.7 GB вместо ~8 GB полной модели

**Как работает:**

1. **Контекст создаётся один раз** и переиспользуется. Перезагрузка контекста стоила бы ~200-500 мс на каждый перевод.

2. **KV-кэш очищается перед каждым запросом.** Qwen3.5 использует M-RoPE (Modified Rotary Position Embeddings), и без сброса позиции сбиваются.

3. **Промпт в формате ChatML:**
```
<|im_start|>system
You are a real-time translator from Russian to English.
Translate the user's speech accurately and naturally.
Output ONLY the translation.<|im_end|>
<|im_start|>user
Translate the following Russian text to English.
Output ONLY the translation, nothing else.

Привет мир<|im_end|>
<|im_start|>assistant
```

4. **Sampling:** temperature=0.1 (почти детерминистический), top_k=20, top_p=0.8 — низкая "креативность" для точного перевода.

5. **Очистка выхода:** удаляются артефакты вроде `<think>`, `<|im_end|>`, кавычки.

```
config/default.toml:
[translation]
model_path = "models/Qwen3.5-4B-Q4_K_M.gguf"
n_gpu_layers = 99        # Все слои на GPU
context_size = 4096       # Достаточно для перевода фраз
max_tokens = 128          # Перевод не длиннее 128 токенов
temperature = 0.1         # Низкая для стабильности
enable_thinking = false   # Без thinking — быстрее
```

### TTS — Text-to-Speech (Phase 4, в разработке)

**Запланированные движки:**
- **Kokoro** — 82M параметров, ONNX, 24kHz, высокое качество
- **Piper** — 15-20M параметров, ONNX, 22050Hz, быстрый

Пока вывод идёт только в консоль. В будущем: переведённый текст будет озвучиваться и выводиться на колонку или виртуальный аудиокабель (VB-Cable).

### Дисплей

В full-режиме консоль показывает две строки, обновляемые в реальном времени:

```
[RU] Привет, как дела?
[EN] Hello, how are you?
```

- Строки обновляются на месте (ANSI escape-коды)
- Partial-транскрипция обновляется пока говоришь
- При завершении фразы строки "коммитятся" и появляется место для следующей

## Структура проекта

```
LocalLLMVoiceProject/
├── setup.sh                    # Установка: зависимости + модели + сборка (Linux)
├── run.sh                      # Запуск переводчика (Linux)
├── build_win.bat               # Сборка под Windows (MSVC + CUDA + Ninja)
├── run_win.bat                 # Запуск полного пайплайна (Windows)
├── ui.bat                      # Запуск desktop-UI (Windows)
├── ui_win.ps1                  # Окно WinForms: живой вывод RU → EN
├── _ui_launch.bat              # Внутренний лаунчер для ui_win.ps1
├── chat.sh                     # Чат с LLM в терминале
├── chat-no-think.sh            # Чат без thinking mode
├── server.sh                   # Локальный OpenAI-совместимый API-сервер
├── config/
│   └── default.toml            # Конфигурация всех компонентов
├── models/                     # Модели (скачиваются setup.sh, не в git)
│   ├── silero_vad.onnx         # VAD — 630 KB
│   ├── gigaam_v3_e2e_ctc_int8.onnx  # STT — 320 MB
│   ├── gigaam_v3_e2e_ctc_tokens.txt # Словарь токенов STT
│   ├── Qwen3.5-4B-Q4_K_M.gguf      # Перевод LLM — 2.7 GB
│   ├── piper-en_US-lessac-medium.onnx      # TTS Piper — 63 MB
│   └── piper-en_US-lessac-medium.onnx.json # Метаданные Piper
├── libs/
│   ├── sherpa-onnx/            # Нативные библиотеки sherpa-onnx (Linux)
│   │   ├── libonnxruntime.so
│   │   ├── libsherpa-onnx-c-api.so
│   │   └── libsherpa-onnx-cxx-api.so
│   └── sherpa-onnx-win/        # Windows DLL + .lib (+ сген. onnxruntime.lib)
├── crates/
│   └── voice-core/             # Основная библиотека и CLI
│       └── src/
│           ├── main.rs          # Точка входа CLI
│           ├── lib.rs           # Экспорт модулей
│           ├── config.rs        # Загрузка TOML-конфига
│           ├── error.rs         # Типы ошибок
│           ├── audio/
│           │   ├── capture.rs   # Захват с микрофона (cpal)
│           │   ├── playback.rs  # Воспроизведение (cpal)
│           │   ├── device.rs    # Выбор аудиоустройств
│           │   └── resample.rs  # Ресемплинг (rubato)
│           ├── vad/
│           │   ├── detector.rs  # Silero VAD обёртка
│           │   └── segment.rs   # Структура речевого сегмента
│           ├── stt/
│           │   ├── gigaam.rs    # GigaAM-v3 через sherpa-onnx
│           │   └── whisper.rs   # Whisper (не используется)
│           ├── translation/
│           │   ├── engine.rs    # Qwen3.5 через llama-cpp
│           │   ├── prompt.rs    # Формирование промптов
│           │   └── streaming.rs # Детекция границ предложений
│           ├── tts/
│           │   ├── kokoro.rs    # Kokoro TTS (заглушка)
│           │   ├── piper.rs     # Piper TTS (заглушка)
│           │   └── phonemize.rs # Фонемизация (заглушка)
│           └── pipeline/
│               ├── orchestrator.rs # Оркестратор пайплайна
│               ├── messages.rs     # Сообщения между потоками
│               └── shutdown.rs     # Координация завершения
├── src-tauri/                  # Tauri десктопное приложение
│   └── src/
│       ├── main.rs             # Точка входа Tauri
│       └── lib.rs              # Tauri-команды (GUI backend)
├── ui/                         # Веб-интерфейс для Tauri
│   ├── index.html
│   ├── main.js
│   └── styles.css
├── tools/
│   ├── llama-cpp/              # llama.cpp CLI (CPU)
│   └── llama-cpp-cuda/         # llama.cpp CLI (CUDA)
└── scripts/
    └── download_models.ps1     # Скачивание моделей (Windows)
```

## Модели

| Компонент | Модель | Размер | Источник | Формат |
|-----------|--------|--------|----------|--------|
| VAD | Silero VAD v5 | 630 KB | [snakers4/silero-vad](https://github.com/snakers4/silero-vad) | ONNX |
| STT | GigaAM-v3 E2E CTC | 320 MB | [Smirnov75/GigaAM-v3-sherpa-onnx](https://huggingface.co/Smirnov75/GigaAM-v3-sherpa-onnx) | ONNX INT8 |
| Перевод | Qwen3.5-4B Q4_K_M | 2.7 GB | [unsloth/Qwen3.5-4B-GGUF](https://huggingface.co/unsloth/Qwen3.5-4B-GGUF) | GGUF |
| TTS | Piper en_US lessac | 63 MB | [rhasspy/piper-voices](https://huggingface.co/rhasspy/piper-voices) | ONNX |

## Конфигурация

Вся конфигурация в `config/default.toml`. Ключевые параметры:

### Аудио
```toml
[audio]
sample_rate = 16000    # 16kHz — стандарт для speech-моделей
channels = 1           # Моно — речь не требует стерео
```

### VAD
```toml
[vad]
threshold = 0.5            # Чувствительность (↑ = строже, меньше ложных)
silence_duration_ms = 1500 # Пауза для конца фразы (↑ = длиннее фразы)
```

### Перевод
```toml
[translation]
n_gpu_layers = 99      # 99 = все слои на GPU. Уменьши если не хватает VRAM
temperature = 0.1      # 0.0-1.0. Ниже = стабильнее перевод
enable_thinking = false # true даёт лучшее качество, но медленнее
```

## Использование LLM отдельно

Модель Qwen3.5-4B можно использовать не только для перевода, но и как полноценного AI-ассистента — для чата или как локальный API-сервер.

### Чат в терминале

```bash
# Обычный чат (с thinking mode — модель "думает" перед ответом)
./chat.sh

# Чат без thinking (быстрее, с системным промптом)
./chat-no-think.sh
```

Это запускает `llama-cli` в интерактивном режиме. Пишешь сообщение — получаешь ответ. Ctrl+C для выхода.

**Параметры в chat.sh:**
```bash
llama-cli \
  -m models/Qwen3.5-4B-Q4_K_M.gguf \  # Модель
  -ngl 99 \                             # Все слои на GPU
  -c 4096 \                             # Контекст 4096 токенов
  -cnv \                                # Conversation mode (история чата)
  --chat-template chatml \              # Формат промпта Qwen
  -t 8                                  # 8 потоков CPU (для слоёв не на GPU)
```

### Локальный API-сервер (OpenAI-совместимый)

```bash
# Запуск на порту 8080 (по умолчанию)
./server.sh

# Или на другом порту
./server.sh 3000
```

Сервер предоставляет **OpenAI-совместимый API** — можно подключить к любому сервису, который умеет работать с OpenAI API (Continue, Open WebUI, LangChain, и т.д.).

**Эндпоинты:**

| Эндпоинт | Описание |
|----------|----------|
| `http://localhost:8080/v1/chat/completions` | Chat Completions (основной) |
| `http://localhost:8080/v1/completions` | Text Completions |
| `http://localhost:8080/v1/models` | Список моделей |
| `http://localhost:8080/health` | Статус сервера |
| `http://localhost:8080/` | Встроенный веб-чат |

**Пример запроса через curl:**
```bash
curl http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "qwen3.5-4b",
    "messages": [
      {"role": "system", "content": "You are a helpful assistant."},
      {"role": "user", "content": "Привет! Расскажи о себе"}
    ],
    "temperature": 0.7
  }'
```

**Пример на Python:**
```python
from openai import OpenAI

client = OpenAI(
    base_url="http://localhost:8080/v1",
    api_key="not-needed"  # Локальный сервер не требует ключ
)

response = client.chat.completions.create(
    model="qwen3.5-4b",
    messages=[
        {"role": "user", "content": "Переведи на английский: Привет мир"}
    ]
)
print(response.choices[0].message.content)
```

**Подключение к сервисам:**

- **Continue (VS Code)** — в настройках: provider = `openai`, base URL = `http://localhost:8080/v1`
- **Open WebUI** — при запуске указать `OLLAMA_BASE_URL=http://localhost:8080`
- **LangChain** — `ChatOpenAI(base_url="http://localhost:8080/v1")`
- **Любой OpenAI-клиент** — заменить `base_url` на `http://localhost:8080/v1`, `api_key` любой

**Параметры сервера:**
```bash
# Полная команда (то что делает server.sh):
LD_LIBRARY_PATH=tools/llama-cpp-cuda \
  tools/llama-cpp-cuda/llama-server \
  -m models/Qwen3.5-4B-Q4_K_M.gguf \
  -ngl 99 \       # GPU layers (99 = все)
  -c 32000 \      # Контекст 32K токенов (для длинных диалогов)
  --port 8080 \   # Порт
  --chat-template chatml  # Формат промпта
```

> **Контекст 32000 vs 4096:** Для перевода в пайплайне достаточно 4096 (переводим короткие фразы). Для чата/сервера ставим 32000 чтобы вмещать длинные диалоги. Больший контекст = больше VRAM.

## Десктопное приложение (Tauri)

Помимо CLI есть GUI на Tauri с веб-интерфейсом:

- Выбор аудиоустройств (микрофон/колонка)
- Переключение режимов (passthrough / stt / full)
- Выбор языков
- Индикатор уровня микрофона в реальном времени
- Тестовый перевод текста

```bash
cargo tauri dev    # Разработка
cargo tauri build  # Сборка
```

## Решения и компромиссы

| Решение | Альтернатива | Почему так |
|---------|-------------|------------|
| Rust | Python | Нужен real-time аудио без GC-пауз. Python не подходит для lock-free ring buffers и zero-alloc callbacks |
| llama.cpp (GGUF) | vLLM, ONNX | GGUF квантизация для consumer GPU. vLLM для серверов. ONNX не поддерживает такие LLM |
| sherpa-onnx | whisper.cpp, ONNX Runtime напрямую | Единый API для VAD + STT. C API с Rust bindings. Поддерживает GigaAM из коробки |
| GigaAM | Whisper | Лучше для русского. Whisper универсальнее но медленнее на русском |
| Синхронные потоки | async/await | CPU-bound задачи (ML инференс). async полезен для I/O-bound |
| rtrb ring buffer | mpsc channel | Lock-free, zero-alloc. Обязательно для real-time аудио callback |
| Crossbeam channel | std mpsc | Bounded + try_recv для drain-to-latest паттерна |
| TOML конфиг | YAML, JSON | Человекочитаемый, нативная поддержка в Rust через serde |

## Лицензия

MIT
