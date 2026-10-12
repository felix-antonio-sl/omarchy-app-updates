# Apps para Omarchy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Instalar en Korax un plugin nativo de Omarchy que compruebe versiones
y actualice aplicaciones de escritorio seleccionadas, excluyendo la suite de IA.

**Architecture:** Un cliente y trabajadores Rust comparten contratos tipados con
un panel QML/Quickshell. Los proveedores consultan sin privilegios y un ejecutor
administrativo Rust prepara y confirma transacciones de paquetes mediante libalpm.
Los trabajos persisten en systemd y la interfaz observa resultados correlacionados.

**Tech Stack:** Rust, Serde/JSON, Clap, Reqwest blocking, Rustix, SHA-256, UUID,
bindgen/pkg-config/libalpm, systemd de usuario, Flatpak, makepkg y QML/QtTest.
Se fijan las dependencias con `Cargo.lock`; la primera ejecución utiliza Rust
1.99.0 y Qt 6.11.2, comprobados en Korax, sin instalar otra herramienta global.

**Spec:** [Especificación aprobada](../specs/2026-10-11-app-updates-design.md).

## Global Constraints

- El nombre visible es **Apps** y el identificador es `korvo.app-updates`.
- El repositorio vive en `/home/korvo/Projects/omarchy-app-updates` en Korax.
- El usuario eligió aplicaciones de escritorio con una lista editable.
- La exclusión del actualizador de IA tiene precedencia sobre la selección.
- Se excluyen los destinos actuales de `updater.py`: `codex`, `claude`, `antigravity`, `opencode`, `hermes`, `claude-desktop` y `chatgpt`.
- Las aplicaciones excluidas no aparecen en la lista gestionada ni en el contador.
- Las consultas de red se limitan a dos proveedores en paralelo.
- Si la caché tiene más de seis horas, se inicia una comprobación al abrir.
- Por defecto no se instala nada automáticamente ni se configura un temporizador permanente.
- Una respuesta `accepted` solo confirma que un trabajo fue recibido.
- No se cancela una operación por cerrar la interfaz.
- Los reinicios requieren una acción explícita en el panel.
- Para Arch se ofrece una actualización completa de paquetes con vista de todos los cambios.
- Las consultas usan bases temporales, nunca una sincronización parcial de la base del sistema.
- La publicación en GitHub o en el catálogo no forma parte de esta especificación.
- Se trabaja en las rutas del usuario; no se editan archivos empaquetados de `/usr/share/omarchy`.

## Review Focus

1. Lanzador con espacios, Unicode, campos de formato y texto parecido a HTML:
   debe descubrirse sin ejecución ni interpretación como contenido activo (T3/T14).
2. La misma app Flatpak instalada para usuario y sistema en ramas distintas:
   no deben mezclarse identidades, commits ni acciones (T6).
3. Reloj adelantado o atrasado y caché de fecha futura: debe volver a comprobarse,
   y un fallo debe conservar el dato como antiguo, nunca como «Al día» (T1/T13).
4. Cancelación de polkit o pérdida del panel mientras se espera una decisión:
   no debe instalarse ni confirmarse con una respuesta antigua (T9/T13/T14).
5. `IgnorePkg`, proveedor cambiado o nuevos paquetes entre plan y ejecución:
   deben revisarse el alcance y la coherencia, sin degradar ni ocultar cambios (T5/T8/T9).

---

## Organización, entorno y contratos comunes

Este es un solo producto: proveedores, política y ejecución comparten las mismas
identidades, planes y operaciones. Se ejecuta de forma secuencial; no hay planes
independientes ni implementaciones paralelas con contratos divergentes.

La ejecución directa por el agente principal es la opción propuesta, con revisión
independiente del cambio completo al final. Antes de escribir código se revisa
este plan y se preserva el método que confirme el usuario. Al ejecutar se aplica
`using-git-worktrees`: crear un worktree desde `main` para la implementación,
salvo que el entorno ya provea aislamiento. El checkout de documentación permanece
limpio hasta integrar el resultado validado.

El PATH del harness no incluye Cargo. En los comandos de este plan, `cargo`
significa añadir `/home/korvo/.cargo/bin` al PATH del proceso, sin modificar la
configuración de la sesión. Usar `CARGO_BUILD_JOBS=2` y evitar compilaciones
concurrentes en el portátil. Leer las instrucciones aplicables al worktree.
Usar `CARGO_TARGET_DIR` fuera del directorio del plugin instalado, en
`~/.local/state/omarchy-app-updates/build/<source-digest>/`, para no introducir
artefactos ni enlaces de compilación en el árbol validado por Omarchy.

### Mapa de archivos previstos

| Ruta | Responsabilidad |
| --- | --- |
| `Cargo.toml`, `Cargo.lock`, `src/lib.rs` | Crate `omarchy-app-updates`, módulos y dependencias fijadas |
| `src/bin/app-updates.rs`, `src/bin/app-updates-helper.rs` | Cliente/trabajador de usuario y entrada administrativa |
| `src/domain/{mod,app,check,plan,operation,error}.rs` | Identidades, reportes, planes, estados y errores |
| `src/protocol.rs`, `src/paths.rs`, `src/store.rs` | JSON v1, rutas y persistencia atómica privada |
| `src/alpm/{mod,bindings,config,read,transaction,callbacks}.rs`, `build.rs`, `native/alpm-wrapper.h` | ABI, configuración de pacman, lectura y transacciones |
| `src/inventory/{mod,desktop,ownership,catalog}.rs` | Descubrimiento XDG y catálogo de instalaciones reconocidas |
| `src/policy.rs`, `catalog/managed-exclusions.json` | Exclusiones de IA y selección efectiva |
| `src/io/{mod,command,http}.rs` | Procesos acotados y descargas HTTPS permitidas |
| `src/providers/{mod,arch,aur,flatpak,releases,telegram,steam,web}.rs` | Adaptadores de comprobación y publicación |
| `catalog/release-sources.json` | Fuentes oficiales y estrategia de extracción por app |
| `src/planner.rs`, `src/helper/{mod,validate,staging,session}.rs` | Plan completo, validación y frontera administrativa |
| `src/execution/{mod,aur,flatpak,telegram,steam,reopen}.rs` | Acciones de usuario y verificación del proveedor |
| `src/jobs/{mod,launch,worker,control,recovery}.rs` | Unidades, bloqueos, decisiones y recuperación |
| `manifest.json`, `ui/{BarWidget,Panel,Controller,AppRow,Details,PlanView,Selection,History,Bootstrap}.qml`, `ui/ViewModel.js` | Plugin y flujos nativos |
| `src/install.rs`, `packaging/{PKGBUILD,org.korvo.app-updates.policy}`, `LICENSE` | Preparación, ejecutor instalado con root y retirada |
| `tests/*.rs`, `tests/fixtures/`, `tests/qml/`, `scripts/verify.sh`, `docs/validation.md` | Contratos, fallos simulados y evidencia sanitizada |

Los DTO se definen en T1; ningún proveedor redefine las identidades. Todos los
errores públicos son `AppError`; `AppResult<T> = Result<T, AppError>`.
`AppId`, `TargetId`, `PlanId` y `OperationId` son tipos nuevos serializables.

- `App`: `id`, `name`, `desktop_ids`, `components: Vec<Target>`, `suggested`.
- `Target`: `id`, `locator: Locator`, `installed: Option<Version>`.
- `Locator`: variantes `Repo`, `Aur`, `Flatpak`, `TelegramLocal`, `SteamClient`,
  `Web`, `Unknown`; incluye paquete, referencia/instalación/rama, ruta o URL según
  corresponda. `Web` conserva `browser: Option<AppId>`.
- `Version`: `raw`, `kind: VersionKind`, `product: Option<String>`; tipos
  `Arch`, `Product`, `FlatpakCommit`.
- `Check`: `target_id`, `checked_at`, `installed`, `available`, `published`,
  `state: CheckState`, `stale`, `source`, `details`. Una publicación desconocida
  no invalida una consulta correcta del canal instalado.
- `CheckState`: `Current`, `UpdateAvailable`, `PublisherAhead`, `InstalledAhead`,
  `Unknown`, `Unsupported`, `WebAutomatic`; QML aplica los textos de la spec.
- `UpdatePlan`: `id`, `created_at`, `expires_at`, `inventory_digest`,
  `policy_digest`, `steps: Vec<PlanStep>`, `digest`. Vigencia inicial: 15 minutos.
- `PlanStep`: `kind: StepKind`, `targets`, `package_changes`, `downloads`,
  `requires_auth`, `reopen`, `aur_recipe: Option<AurRecipeRef>`.
  `AurRecipeRef` contiene `package_base`, `commit`, `digest`.
  Tipos: `ArchFull`, `AurBuild`, `FlatpakUpdate`,
  `TelegramReplace`, `SteamUpdate`, `Reopen`.
- `PackageChange`: `name`, `old_version`, `new_version`, `repository`,
  `reason`, `download_bytes`; `reason` distingue app, dependencia y sistema.
- `Download`: `url`, `expected_sha256: Option<String>`, `bytes: Option<u64>`.
- `Operation`: `id`, `plan_digest`, `phase: Phase`, `unit`, `events`, `results`.
- `Phase`: `Prepared`, `Queued`, `Downloading`, `Building`, `AwaitingAuth`,
  `AwaitingDecision`, `Installing`, `Verifying`, `Completed`, `Failed`, `Cancelled`.
- `TargetResult`: `target_id`, `outcome: Outcome`, `old_version`, `new_version`,
  `error`; `Outcome`: `Updated`, `Current`, `NeedsReopen`, `Failed`, `Skipped`.
- `OperationEvent`: `operation_id`, `seq`, `phase`, `message`, `progress`,
  `detail: Option<EventDetail>`; `EventDetail` contiene `PlanRevised(UpdatePlan)`,
  `Question { id: String, kind: String, choices: Vec<String> }` o `NeedsReopen(AppId)`.
- `ErrorCode`: al menos `ProtocolMismatch`, `InvalidInput`, `Unsupported`,
  `IoError`, `NetworkError`, `Busy`, `CoverageNeedsReview`, `PlanExpired`,
  `PlanChanged`, `AuthCancelled`, `AuthUnavailable`, `AbiIncompatible`,
  `NeedsEnginePreparation`, `UnsafeArtifact`, `TransactionFailed`.
- `UserConfig`: `schema_version: 1`, `selected: Vec<AppId>`, `presentation`.
- `Coverage`: `Verified` o `NeedsReview`, con motivos y destinos desconocidos.
- `Report`: `apps`, `checks`, `selected`, `coverage: Coverage`, `operation`, `checked_at`.
- `State`: `config`, `report`, `plans`, `operations`; colecciones por ID.

`Paths::for_home(home: PathBuf) -> Paths` fija configuración, estado y plugin
según la spec; el ejecutor administrativo vive en
`/usr/lib/omarchy-app-updates/app-updates-helper`. `Paths` también conserva
`pacman_conf`, `system_db` y `runtime_dir` para adaptadores y pruebas aisladas.

## Tareas de implementación

### T1: Contratos, CLI y persistencia verificables

**Files:** crear `Cargo.toml`, `Cargo.lock`, `src/lib.rs`, `src/bin/app-updates.rs`,
`src/domain/*.rs`, `src/{protocol,paths,store}.rs`, `tests/contract.rs`,
`tests/fixtures/protocol-v1.json`, `LICENSE`.

**Interfaces:** produce DTO anteriores; `decode_request(bytes: &[u8]) -> AppResult<Request>`;
`dispatch(request: Request, paths: &Paths) -> AppResult<Response>`;
`Store::load(paths: &Paths) -> AppResult<State>`;
`Store::save(&self, state: &State) -> AppResult<()>`;
`should_refresh(checked_at: Option<u64>, now: u64, force: bool) -> bool`.
`Request/Response` incluyen `protocol_version: 1` y `request_id`; las variantes
de Request son las operaciones de la spec, más `Worker`, `PrepareEngine` y
`RemovePlugin` internos. Una operación no implementada responde `Unsupported`.
El sobre usa `action` en snake_case y `payload` tipado por acción; respuestas
usan `result` o `error`, nunca un campo de comandos libres.

- [ ] **1. Preparar el crate mínimo para ejecutar y escribir `protocol_and_private_state`**
  en `tests/contract.rs`, con librería inicialmente vacía:
  JSON v2 y campos de acción desconocidos se rechazan; respuesta mantiene ID;
  guardar/cargar conserva selección; un archivo dañado devuelve error sin
  sobrescribirlo. Verificar modos 0700/0600, copia anterior y ausencia de temporales.
  Añadir `refresh_clock_edges` con estas aserciones:
  ```rust
  assert!(!should_refresh(Some(0), 21_600, false));
  assert!(should_refresh(Some(0), 21_601, false));
  assert!(should_refresh(Some(100), 99, false));
  assert!(should_refresh(Some(100), 100, true));
  ```
- [ ] **2. Ejecutar** `cargo test --test contract`: debe fallar por los contratos
  ausentes, no por un Cargo.toml inválido; conservar esa evidencia antes de implementar.
- [ ] **3. Implementar** las interfaces en sus archivos, binario con stdin JSON y
  stdout JSON, mensajes de diagnóstico en stderr, reemplazo atómico y esquema v1.
  Fijar licencia `GPL-3.0-or-later` y conservar avisos de dependencias.
- [ ] **4. Ejecutar** `cargo test --test contract` y `cargo fmt --check`: deben pasar.
- [ ] **5. Commit:** `feat: add app update contracts and private state`.

### T2: Integración de libalpm y lectura de paquetes

**Files:** crear `build.rs`, `native/alpm-wrapper.h`, `src/alpm/{mod,bindings,config,read}.rs`,
`tests/alpm_read.rs`; modificar `Cargo.toml`, `Cargo.lock`, `src/lib.rs`.

**Interfaces:** produce `AlpmConfig::load(path: &Path, io: &CommandRunner) -> AppResult<AlpmConfig>`;
`AlpmReader::open(config: &AlpmConfig, dbpath: &Path) -> AppResult<AlpmReader>`;
`AlpmReader::installed(&self) -> AppResult<Vec<PackageFacts>>`;
`AlpmReader::owner(&self, path: &Path) -> AppResult<Option<String>>`;
`arch_compare(a: &str, b: &str) -> AppResult<Ordering>`.
`PackageFacts` conserva nombre, versión, motivo explícito/dependencia, archivos,
URL y dependencias. `CommandRunner` de `src/io/command.rs` expone
`run(&self, spec: &CommandSpec) -> AppResult<Captured>`; `CommandSpec` contiene
programa absoluto, argv, env permitido y timeout; `Captured` contiene código,
stdout y stderr. Crear aquí esa abstracción, ampliada en T5.
`CommandBackend::run(&self, spec: &CommandSpec) -> AppResult<Captured>` permite
inyectar procesos simulados mediante `CommandRunner::with_backend` en pruebas.

- [ ] **1. Escribir `native_abi_and_package_read`**: comparar `1:1.4.43-3` con
  `1.4.44-1` respeta epoch; revisión `1.3.1-2` supera `1.3.1-1`; leer una base
  sintética sin tocar `/var/lib/pacman`; propietario por archivo correcto;
  `Include` y orden de repositorios se preservan; discrepancia de ABI se rechaza.
- [ ] **2. Ejecutar** `cargo test --test alpm_read`: falla por interfaces ausentes.
- [ ] **3. Implementar** generación de bindings del header instalado con bindgen
  y wrappers RAII en `src/alpm/`, con `unsafe` limitado a este módulo. Consultar
  configuración con `/usr/bin/pacman-conf`; no sustituirla por un parser incompleto.
  Comparar major de ABI compilada y cargada antes de abrir un handle. No adoptar
  `alpm 5.0.2` suponiendo soporte de libalpm 16: su documentación declara 15.
- [ ] **4. Ejecutar** `cargo test --test alpm_read`: PASS. Prueba de solo lectura
  con la libalpm 16.0.1 instalada; cualquier incompatibilidad bloquea mutaciones.
- [ ] **5. Commit:** `feat: read Arch package facts through verified libalpm bindings`.

### T3: Inventario XDG y catálogo de instalaciones

**Files:** crear `src/inventory/{mod,desktop,ownership,catalog}.rs`,
`tests/inventory.rs`, `tests/fixtures/desktop/`; modificar `src/lib.rs`, CLI.

**Interfaces:** consume `Paths`, `PackageFacts`, `AlpmReader`;
produce `discover(paths: &Paths, facts: &[PackageFacts], io: &CommandRunner) -> AppResult<Vec<App>>`;
`parse_desktop(id: &str, bytes: &[u8]) -> AppResult<Option<DesktopEntry>>`.
`DesktopEntry` contiene ID, nombre, Exec como dato, icono, ocultación y terminal.

- [ ] **1. Escribir `xdg_inventory_without_execution`**: precedencia usuario,
  `Hidden` elimina entrada del sistema, `NoDisplay` no ofrece app, varios
  lanzadores de LibreOffice cuentan una vez, dos instalaciones locales no se
  fusionan. Fixture con nombre `Clínica <b>LE</b>`, ruta con espacios y `%U`:
  se conserva el nombre literal y un ejecutable marcador nunca se ejecuta.
  La versión `1.0` del desktop WhatsApp no se convierte en versión de producto.
- [ ] **2. Ejecutar** `cargo test --test inventory`: falla antes de implementar.
- [ ] **3. Implementar** interfaces y resolución de propiedad sin ejecutar Exec.
  Leer lista base de Omarchy como sugerencia; reconocer las 12 apps iniciales,
  Telegram local, Steam cliente y webapps. Orca ausente no se reconstruye.
- [ ] **4. Ejecutar** `cargo test --test inventory`: PASS; `inventory` real debe
  devolver solo datos y no abrir aplicaciones. Evidencia real en estado privado.
- [ ] **5. Commit:** `feat: discover desktop applications and installation identities`.

### T4: Exclusiones obligatorias y selección editable

**Files:** crear `src/policy.rs`, `catalog/managed-exclusions.json`,
`tests/policy.rs`; modificar `src/store.rs`, CLI.

**Interfaces:** produce `Policy::load(updater_source: &Path, catalog: &Path) -> AppResult<Policy>`;
`Policy::visible(&self, apps: &[App]) -> Vec<App>`;
`Policy::selected(&self, apps: &[App], config: &UserConfig) -> AppResult<Vec<AppId>>`;
`Policy::digest(&self) -> String`; consume `Coverage` definido en T1.

- [ ] **1. Escribir `managed_tools_never_selected`**: siete destinos y alias
  excluidos aun presentes en config; fuente Python se lee sin ejecutar un
  marcador; destino nuevo o sintaxis no reconocida produce `NeedsReview`;
  seleccionar app base es posible; Hermes Desktop independiente queda sin
  selección inicial y nunca autoriza su CLI; app retirada deja de contar.
- [ ] **2. Ejecutar** `cargo test --test policy`: falla por Policy ausente.
- [ ] **3. Implementar** lector limitado del literal TARGETS y mapa de rutas,
  nombres y lanzadores, sin importar Python. Aplicar política al inventario,
  comprobaciones y cambios de selección; guardar config con Store de T1.
- [ ] **4. Ejecutar** `cargo test --test policy`: PASS. Cambiar cobertura en una
  fixture bloquea planes de instalación sin consultar ninguna herramienta IA.
- [ ] **5. Commit:** `feat: enforce AI updater exclusions and editable app selection`.

### T5: Consultas de repositorios y AUR

**Files:** crear `src/providers/{mod,arch,aur}.rs`, `src/io/{mod,http}.rs`,
`tests/package_checks.rs`; ampliar `src/io/command.rs`, `src/alpm/read.rs`.

**Interfaces:** produce `ProviderContext { paths, commands, http, now }`;
trait `Provider::check(&self, target: &Target, ctx: &ProviderContext) -> AppResult<Check>`;
`ArchProvider`, `AurProvider`; `HttpClient::get(&self, url: &str, policy: &DownloadPolicy) -> AppResult<HttpResponse>`.
`DownloadPolicy` fija hosts, tamaño y redirects; `HttpResponse` incluye URL final,
status, bytes y cabeceras. `RepoSnapshot` contiene repositorios ordenados,
paquetes disponibles y fingerprint; `sync_snapshot(ctx: &ProviderContext) -> AppResult<RepoSnapshot>`.
`HttpTransport::send(&self, url: &str, policy: &DownloadPolicy) -> AppResult<HttpResponse>`
permite un transporte simulado; `HttpClient::with_transport` conserva las mismas
validaciones HTTPS/host incluso sin una conexión de red real.

- [ ] **1. Escribir `repo_priority_and_unknown_sources`**: Arch extra preferido
  sobre revisión mayor de Omarchy; salida 2 de checkupdates equivale a consulta
  sin pendientes; caída de espejo equivale a Unknown; paquete en repo no consulta
  AUR; desaparecido del repo se ofrece con origen nuevo, sin cambiarlo al instalar;
  `IgnorePkg` queda visible como restricción. AUR RPC no ejecuta PKGBUILD.
- [ ] **2. Ejecutar** `cargo test --test package_checks`: FAIL antes de adaptadores.
- [ ] **3. Implementar** base temporal exclusiva por operación con checkupdates,
  lectura de todos los paquetes disponibles y RPC AUR. Procesos: timeout 60 s;
  HTTP: timeout 30 s; máximo tres intentos solo para fallos transitorios.
  No ejecutar sincronización del sistema, yay, makepkg ni `--version` de apps.
- [ ] **4. Ejecutar** `cargo test --test package_checks`: PASS; snapshot real
  deja intactos hashes y mtimes de la base sync del sistema.
- [ ] **5. Commit:** `feat: check configured repositories and AUR without installing`.

### T6: Identidad y proveedor Flatpak

**Files:** crear `src/providers/flatpak.rs`, `src/execution/{mod,flatpak}.rs`,
`tests/flatpak.rs`; ampliar `src/inventory/catalog.rs`.

**Interfaces:** `FlatpakProvider` implementa Provider;
`FlatpakInstall` distingue `User` y `System { name: String }`;
`run_flatpak(step: &PlanStep, ctx: &ExecutionContext) -> AppResult<Vec<TargetResult>>`.
`ExecutionContext` de `src/execution/mod.rs` contiene ProviderContext, Paths,
OperationId y `emit: &dyn EventSink`; `EventSink::send(event: OperationEvent) -> AppResult<()>`.

- [ ] **1. Escribir `flatpak_scope_and_commits`**: app igual instalada en user y
  system, ramas master/stable y remotos diferentes producen IDs y comandos
  distintos; commit nuevo sin versión sí detecta actualización; coincidencia
  confirma Current; runtime necesario está en el plan; runtime EOL se informa,
  sin migrarlo ni borrar permisos. Update que termina sin nuevo commit no dice Updated.
- [ ] **2. Ejecutar** `cargo test --test flatpak`: FAIL antes del proveedor.
- [ ] **3. Implementar** interfaces usando columnas explícitas y argumentos de
  Flatpak por instalación/referencia; exigir mismo canal y verificar commit final.
  El método de ejecución solo acepta PlanStep validado por T8, jamás argv del panel.
- [ ] **4. Ejecutar** `cargo test --test flatpak`: PASS; comparar en modo lectura
  GeForce NOW con su remoto oficial. No hacer una actualización para este check.
- [ ] **5. Commit:** `feat: check and update scoped Flatpak references`.

### T7: Publicaciones oficiales, Telegram, Steam y webapps

**Files:** crear `src/providers/{releases,telegram,steam,web}.rs`,
`catalog/release-sources.json`, `tests/releases.rs`, `tests/fixtures/releases/`.

**Interfaces:** providers TelegramProvider, SteamProvider y WebProvider;
`published_version(target: &Target, ctx: &ProviderContext) -> AppResult<Option<Version>>`;
`classify(installed: &Version, available: Option<&Version>, published: Option<&Version>) -> AppResult<CheckState>`.
Telegram consulta metadatos/recibos y catálogo oficial; Steam lee manifiesto del
canal; Web vincula el navegador actual sin ejecutar el acceso.

- [ ] **1. Escribir `publisher_and_install_channel_are_distinct`**: estable frente
  a prerelease; publicación mayor pero paquete al día da PublisherAhead; versión
  instalada posterior no ofrece downgrade; JSON alterado/429 da detalle de
  publicación desconocida sin convertir canal comprobado en fallo; redirect fuera
  del allowlist y descarga HTTP se rechazan. Steam paquete y cliente quedan distintos;
  remotos inexistentes, navegador desconocido y metadatos Telegram ausentes no se inventan.
- [ ] **2. Ejecutar** `cargo test --test releases`: FAIL antes de adaptadores.
- [ ] **3. Implementar** fuentes oficiales verificadas durante la ejecución:
  Chrome Version History; Mozilla product-details Thunderbird; endpoint update
  de VS Code; GitHub oficial de Ghostty/Audacity/Protégé/Telegram/Voxtype;
  GitLab oficial de Remmina; manifiesto Valve del canal. Obtener URL de Voxtype
  del paquete real y verificarla. TickTick solo usa fuente oficial si permite
  extraer versión fiable; en otro caso su publicación queda sin cobertura.
  GeForce NOW utiliza el remoto oficial ya asociado. Registrar fixtures pequeñas,
  origen y estrategia; no copiar artículos ni datos de usuario.
- [ ] **4. Ejecutar** `cargo test --test releases`: PASS; comparar app/package
  según el tipo de versión y devolver fuentes y fecha en Check.
- [ ] **5. Commit:** `feat: distinguish official releases from installable app versions`.

### T8: Planes, fingerprints y alcance de sistema

**Files:** crear `src/planner.rs`, `tests/plans.rs`; ampliar `src/domain/plan.rs`.

**Interfaces:** `build_plan(report: &Report, selection: &[AppId], snapshot: &RepoSnapshot, policy: &Policy, now: u64) -> AppResult<UpdatePlan>`;
`validate_plan(plan: &UpdatePlan, report: &Report, policy: &Policy, now: u64) -> AppResult<()>`;
`plan_digest(plan: &UpdatePlan) -> String`.

- [ ] **1. Escribir `reviewed_scope_is_complete`**: app Arch seleccionada genera
  paso ArchFull con todas las actualizaciones necesarias; paquetes de sistema y
  locales auxiliares quedan identificados; idioma Thunderbird se preserva;
  dependencia nueva o versión distinta invalida digest; vence a 900 s; selección
  Unknown no produce instalación; Web no instala; cobertura NeedsReview bloquea;
  IgnorePkg incompatible con coherencia bloquea, sin quitar la restricción.
  Un paquete del alcance completo que toque un destino/ruta de IA protegido
  bloquea el plan completo, sin ocultar ese paquete ni hacer upgrade parcial.
- [ ] **2. Ejecutar** `cargo test --test plans`: FAIL antes del planificador.
- [ ] **3. Implementar** interfaces y digest canónico de destinos, versiones,
  dependencias, receta AUR y decisiones, con listas ordenadas. No hashear solo
  el nombre de la app. Ordenar sistema coherente, compilaciones y acciones locales;
  excluir mise, limpieza de huérfanos y hooks generales de Omarchy.
- [ ] **4. Ejecutar** `cargo test --test plans`: PASS. Snapshot y migraciones
  específicas de Omarchy son partes visibles del plan si corresponden.
- [ ] **5. Commit:** `feat: prepare complete update plans and reject changed scopes`.

### T9: Ejecutor administrativo y transacciones libalpm

**Files:** crear `src/bin/app-updates-helper.rs`, `src/helper/{mod,validate,staging,session}.rs`,
`src/alpm/{transaction,callbacks}.rs`, `tests/helper.rs`, `tests/transactions.rs`.

**Interfaces:** produce `HelperRequest::{Prepare { operation_id, plan: UpdatePlan }, Confirm { operation_id, digest }, Answer { operation_id, question_id, answer }, Abort { operation_id }}`;
`HelperEvent::{Prepared, RevisedPlan, Question, Progress, Result}`;
`DecisionValue::{Approve, Reject, SelectProvider(String)}` identifica respuestas
admitidas y `QuestionId` correlaciona una pregunta dentro de la operación;
`HelperSession::run(input: &mut dyn BufRead, output: &mut dyn Write) -> AppResult<()>`;
`HelperClient::start(operation_id: &OperationId, paths: &Paths) -> AppResult<HelperClient>`;
`HelperClient::prepare(&mut self, plan: &UpdatePlan) -> AppResult<HelperEvent>`;
`HelperClient::confirm(&mut self, digest: &str) -> AppResult<()>`;
`HelperClient::answer(&mut self, question: &QuestionId, value: &DecisionValue) -> AppResult<()>`;
`HelperClient::next_event(&mut self) -> AppResult<HelperEvent>`;
trait `PackageTransaction` con
`prepare(&mut self, approved: &UpdatePlan) -> AppResult<UpdatePlan>`,
`commit(&mut self, digest: &str, sink: &dyn EventSink) -> AppResult<Vec<TargetResult>>`
y `release(&mut self) -> AppResult<()>`;
`AlpmTransaction` implementa el trait. `stage_artifact(path: &Path, expected: &str, caller_uid: u32) -> AppResult<PathBuf>` copia y verifica en staging root.

- [ ] **1. Escribir `privileged_boundary_and_transaction_decisions`**: request
  desconocido, código ABI incompatible, path de usuario para programa auxiliar,
  symlink en staging, checksum sustituido y root sin caller válido se rechazan;
  preparar no instala; plan revisado no confirma con digest anterior; pregunta
  de conflicto, reemplazo o llave no recibe una respuesta por defecto.
  El helper vuelve a comprobar exclusiones en el alcance resuelto, incluidos
  archivos de paquetes; no confía solo en la selección filtrada del cliente.
  Cancelar polkit devuelve Cancelled sin commit ni resultado histórico exitoso.
  Configuración administrativa recibida desde rutas de usuario se rechaza;
  la operación se enlaza desde Prepare y no admite Confirm de otro ID.
- [ ] **2. Ejecutar** `cargo test --test helper --test transactions`: FAIL con
  motor administrativo simulado, nunca usando una transacción real del host.
- [ ] **3. Implementar** interfaces, lectura solo de configuración root confiable,
  locks Omarchy/pacman, caller de pkexec y polkit sin agente textual fallback.
  Conservar SigLevel/GPGDir, hooks normales de pacman, configuración y razones
  de instalación. No permitir nodeps, dbonly, overwrite arbitrario o degradación.
  Preparar, listar cambios, pedir decisión correlacionada y confirmar usando
  libalpm; callbacks solo emiten datos, nunca responden por intuición.
  Timeout de decisión: 300 s antes de commit; liberar transacción al rechazar.
  Snapshot antes de cambios; informar ausencia/fallo. Migraciones de Omarchy
  aprobadas y compatibles se ejecutan por ID fijo; si no puede preservarse la
  exclusión, detener con detalle. No ejecutar `omarchy update` completo.
- [ ] **4. Ejecutar** tests de T9 y `cargo clippy --all-targets -- -D warnings`:
  PASS; confirmar contadores cero de commit en todos los rechazos.
- [ ] **5. Commit:** `feat: authorize and confirm native package transactions`.

### T10: Revisión y compilación AUR sin root

**Files:** crear `src/execution/aur.rs`, `tests/aur_build.rs`; ampliar AurProvider,
PlanStep y staging administrativo.

**Interfaces:** `prepare_aur(target: &Target, ctx: &ProviderContext) -> AppResult<AurRecipe>`;
`AurRecipe` incluye package base, commit, archivos de receta, diff, dependencias
y digest; `build_aur(recipe: &AurRecipe, ctx: &ExecutionContext) -> AppResult<Vec<BuiltArtifact>>`;
`BuiltArtifact` contiene nombre, versión, ruta y SHA-256.

- [ ] **1. Escribir `aur_review_pins_execution`**: consulta no ejecuta receta;
  commit cambiado bloquea build; root no compila; dependencias de build figuran
  en plan; package base dividido solo instala destinos aprobados; fallo de build
  no se reintenta como instalación; artefacto con nombre inesperado se rechaza.
  Una versión o hash de artefacto diferente exige actualizar el plan y revisarlo.
- [ ] **2. Ejecutar** `cargo test --test aur_build`: FAIL antes del ejecutor.
- [ ] **3. Implementar** interfaces, checkout privado del commit revisado y
  makepkg sin root ni instalación automática. Leer `.SRCINFO` sin regenerarlo
  durante consulta; antes de compilar revisar todos los archivos ejecutables de
  la receta y sus dependencias AUR transitivas. Una dependencia con receta no
  revisada impide continuar. Incluir AurRecipeRef y metadatos/hash de artefactos
  en el plan que acepta el usuario antes de instalar por helper, tras coherencia
  de sistema. No reutilizar la aprobación de compilación como aprobación de un
  paquete con versión o contenido inesperado.
- [ ] **4. Ejecutar** `cargo test --test aur_build`: PASS con recetas de prueba;
  sistema actualizado seguido de build fallido produce resultados por fase.
- [ ] **5. Commit:** `feat: review pinned AUR recipes and build as the user`.

### T11: Cambio de Telegram conservando su instalación y datos

**Files:** crear `src/execution/{telegram,reopen}.rs`, `tests/telegram_activation.rs`.

**Interfaces:** `stage_telegram(step: &PlanStep, ctx: &ExecutionContext) -> AppResult<StagedTelegram>`;
`activate_telegram(stage: &StagedTelegram, ctx: &ExecutionContext) -> AppResult<TargetResult>`;
`running_apps(app: &App, paths: &Paths) -> AppResult<Vec<RunningApp>>`;
`RunningApp` conserva PID, exe, start time e identidad del binario;
`request_reopen(app: &App, approval: &ReopenApproval, ctx: &ExecutionContext) -> AppResult<()>`.

- [ ] **1. Escribir `telegram_preserves_profile_and_previous_binary`**: archive
  con traversal o symlink externo rechazado; instalación activa exige decisión;
  tdata y lanzador conservan hashes; binario previo retenido; activación fallida
  restaura instalación; proceso con mismo PID reutilizado nunca se cierra;
  versión igual no produce cierre. No tratar hash propio como firma oficial.
- [ ] **2. Ejecutar** `cargo test --test telegram_activation`: FAIL con staging
  y procesos simulados, sin tocar `~/.local/share/TelegramDesktop` real.
- [ ] **3. Implementar** interfaces con origen permitido y extracción restringida,
  preservación de Telegram/Updater y symlink externo existente del lanzador.
  Antes de reemplazar verificar proceso y aprobación; después verificar versión
  del binario/metadatos y funcionamiento sin exigir una nueva autenticación.
- [ ] **4. Ejecutar** `cargo test --test telegram_activation`: PASS; ningún test
  escribe el perfil real ni abre Telegram para descubrir su versión.
- [ ] **5. Commit:** `feat: update local Telegram while preserving account data`.

### T12: Cliente Steam y estado de reapertura

**Files:** crear `src/execution/steam.rs`, `tests/steam_execution.rs`;
ampliar `src/execution/reopen.rs`.

**Interfaces:** `run_steam(step: &PlanStep, ctx: &ExecutionContext) -> AppResult<TargetResult>`;
`needs_reopen(app: &App, processes: &[RunningApp]) -> AppResult<bool>`.

- [ ] **1. Escribir `steam_client_is_not_launcher_or_games`**: juegos activos o
  estado incierto bloquean cliente; Steam cerrado permite actualización;
  éxito de lanzamiento sin manifiesto nuevo no equivale a Updated; comparación
  conserva canal y build; archivos de juegos/userdata no cambian. Un navegador
  con exe eliminado muestra NeedsReopen y no se cierra por instalar su paquete.
- [ ] **2. Ejecutar** `cargo test --test steam_execution`: FAIL antes del runner.
- [ ] **3. Implementar** actualización oficial con flags comprobados del Steam
  instalado, unidad supervisada y verificación de manifiesto. Esperar cierre
  manual si no se puede confirmar compatibilidad; no enviar señales a juegos.
  Verificar reapertura por inode/binario y PID/start time, sin usar solo clase UI.
- [ ] **4. Ejecutar** `cargo test --test steam_execution`: PASS. Paquete y cliente
  se registran por componente; el contador agrupa la aplicación.
- [ ] **5. Commit:** `feat: verify Steam client updates and pending app reopen`.

### T13: Coordinación, trabajos persistentes y recuperación

**Files:** crear `src/jobs/{mod,launch,worker,control,recovery}.rs`,
`tests/jobs.rs`; ampliar CLI, ProviderContext, Store y ExecutionContext.

**Interfaces:** `submit(plan: &UpdatePlan, paths: &Paths) -> AppResult<OperationId>`;
`run_worker(operation_id: &OperationId, paths: &Paths) -> AppResult<()>`;
`respond(operation_id: &OperationId, decision: &UserDecision, paths: &Paths) -> AppResult<()>`;
`recover(paths: &Paths) -> AppResult<Vec<Operation>>`;
`refresh(report: &Report, policy: &Policy, ctx: &ProviderContext) -> AppResult<Report>`.
`UserDecision` incluye `question_id: QuestionId`, `digest` y
`value: DecisionValue` de T9, sin credenciales.

- [ ] **1. Escribir `jobs_survive_panel_and_correlate_results`**: cierre de cliente
  no termina worker; reload no sustituye exe del trabajo; segunda instalación
  devuelve Busy; fallo polkit no sobreescribe resultado de otra operación;
  decisión tardía/ID ajeno se rechaza; después de caída se coteja unidad y proveedor;
  accepted nunca es Completed. Fuente caída conserva caché stale; máximo dos
  proveedores concurrentes; no hay timer instalado ni proceso permanente idle.
- [ ] **2. Ejecutar** `cargo test --test jobs`: FAIL con systemd y reloj simulados.
- [ ] **3. Implementar** ejecutable copiado por job, bloqueo global y unidad
  `omarchy-app-updates-<operation-id>.service`, state privado y socket de decisiones
  en runtime con modo 0600. Secuencia por fases, resultado por target, eventos
  monotónicos y respuesta del helper vinculada al último plan. Permitir cancelar
  preparación; durante commit solo cerrar/observar. No matar worker desde QML.
  Si cambia la ABI de libalpm durante ArchFull, conservar su resultado y detener
  nuevas transacciones con `NeedsEnginePreparation`; los pasos restantes se
  muestran pendientes para un plan nuevo tras preparar el motor.
- [ ] **4. Ejecutar** `cargo test --test jobs`: PASS. Integración con unidad real
  usando solo proveedor simulado que duerme brevemente y genera un recibo nuevo.
- [ ] **5. Commit:** `feat: persist update jobs and recover verified operation state`.

### T14: Panel nativo, selección, planes y errores

**Files:** crear `manifest.json`, `ui/*.qml`, `ui/ViewModel.js`,
`tests/qml/tst_AppUpdates.qml`, `tests/fixtures/ui-report.json`.

**Interfaces:** BarWidget expone `opened`, `open()`, `close()`, `toggle()`,
`closeForPopoutSwitch()`; Controller expone `report`, `busy`, `error`,
`refresh(force)`, `saveSelection(ids)`, `preparePlan(ids)`, `applyPlan(digest)`,
`answer(operationId, questionId, digest, value)` y `prepareEngine()`.
`ViewModel.js` expone `visibleRows(report, filter, query)` y `updateCount(report)`;
el binario y bootstrap se referencian con rutas relativas verificadas al plugin.

- [ ] **1. Escribir `tst_AppUpdates.qml`**: fixture con update, PublisherAhead,
  Unknown, WebAutomatic y Steam doble da contador correcto; filtro/búsqueda
  preservan IDs; texto `<b>` se ve literal; error/fecha stale visibles; plan
  muestra paquetes adicionales; cancelación de auth y plan revisado no activan
  apply viejo; Escape cierra y reabrir conserva operación. Testear selection e
  historial con Controller falso que captura solicitudes, sin ejecutar binarios.
- [ ] **2. Ejecutar** `/usr/lib/qt6/bin/qmltestrunner -input tests/qml -import /usr/share/omarchy/shell`:
  debe fallar antes de crear los componentes.
- [ ] **3. Implementar** interfaces con `qs.Ui`/`qs.Commons`, tamaño adaptativo,
  scroll, foco, switchPanel, lista editable, detalles, receta AUR, plan, decisiones
  e historial. Texto externo siempre PlainText; enlaces solo a fuentes permitidas.
  En abrir, mostrar caché y refrescar si corresponde; durante job observar sin
  hacer comprobaciones pesadas cada segundo. Mantener idioma español de estados.
- [ ] **4. Ejecutar** QtTest anterior, `omarchy plugin validate .` y
  `/usr/lib/qt6/bin/qmllint -I /usr/share/omarchy/shell ui/*.qml`: PASS sin errores
  nuevos; clasificar avisos propios de imports del shell con evidencia.
- [ ] **5. Commit:** `feat: add native Omarchy application update panel`.

### T15: Preparación gráfica, instalación y retirada

**Files:** crear `src/install.rs`, `packaging/PKGBUILD`,
`packaging/org.korvo.app-updates.policy`, `tests/install.rs`;
ampliar `ui/Bootstrap.qml`, CLI, README y Cargo metadata.

**Interfaces:** `prepare_engine(plugin: &Path, paths: &Paths, sink: &dyn EventSink) -> AppResult<InstallReceipt>`;
`install_plugin(source: &Path, paths: &Paths) -> AppResult<InstallReceipt>`;
`remove_plugin(paths: &Paths) -> AppResult<InstallReceipt>`.
`InstallReceipt` conserva versión, digest de artefactos, archivos propios y backup
de shell.json. `engine_compatible(plugin_version: &str, helper_version: &str, protocol: u32, abi_major: u32) -> bool`.

- [ ] **1. Escribir `install_is_atomic_and_reversible`**: fuente/staging con
  symlinks externos rechazada; engine ausente ofrece preparar; mismatch bloquea
  apply; build requiere Cargo.lock y no se repite al abrir; error antes de swap
  conserva plugin; shell backup preserva los widgets; job activo bloquea retirada;
  uninstall deja perfiles y apps intactos; todos los archivos admin son root-owned.
  Si una nueva ABI impide arrancar el binario previo, Bootstrap todavía permite
  recompilar; un job activo conserva su ejecutable y no se reemplaza a mitad.
- [ ] **2. Ejecutar** `cargo test --test install`: FAIL con filesystem y comandos
  aislados; los tests no escriben `~/.config/omarchy` ni `/usr` reales.
- [ ] **3. Implementar** build locked con dos jobs, progreso y preparación explícita.
  Bootstrap puede lanzar la compilación directamente en una unidad systemd con
  programa Cargo confiable y argv fijo aunque el motor no arranque. No depende
  del motor roto para ejecutar `PrepareEngine`. Mantener CARGO_TARGET_DIR fuera
  del plugin y detectar terminación por unidad/recibo, no por cerrar el panel.
  Empaquetar helper/policy como paquete local `omarchy-app-updates-helper`, con
  rutas propias y dependencias ya instaladas verificadas. Instalar ese artefacto
  local mediante `/usr/bin/pkexec --disable-internal-agent /usr/bin/pacman -U`;
  esta fase bootstrap se identifica como instalación del plugin, no como update
  de apps. No ejecutar una sincronización parcial para instalarlo.
  Policy solo autoriza el path fijo del helper con `auth_admin`, sin passwordless.
  Toda interacción de pacman durante bootstrap debe tener resolución en GUI;
  rechazar conflictos inesperados y conservar la instalación previa, sin dejar
  un prompt esperando stdin ni responder automáticamente a eliminaciones.
  Copiar a staging de usuario solo manifiesto, QML, catálogo, fuentes necesarias
  para preparación y binario; excluir target, symlinks y datos del host. Swap
  completo, rescan, enable y verificar IPC. Retirar helper por su paquete propio.
- [ ] **4. Ejecutar** `cargo test --test install`: PASS; revisar lista del paquete
  local antes de cualquier instalación. README explica que plugin add no compila.
- [ ] **5. Commit:** `feat: prepare and install the Rust plugin through graphical auth`.

### T16: Verificación completa y aceptación en Korax

**Files:** crear `scripts/verify.sh`, `tests/acceptance.rs`, `docs/validation.md`;
actualizar README y seguimiento de este plan.

**Interfaces:** `scripts/verify.sh` ejecuta una vez verificaciones Rust/QML sin
mutar instalaciones de apps; `acceptance.rs` usa adaptadores simulados y paths
temporales. Evidencia real va a `~/.local/state/omarchy-app-updates/acceptance/`;
el informe Git solo conserva conclusiones sanitizadas y limitaciones.

- [ ] **1. Escribir `whole_flow_in_isolated_environment`**: descubrir, excluir,
  seleccionar, comprobar, preparar, aceptar, observar y verificar un trabajo;
  cambiar alcance exige revisión; cerrar UI no cancela; perfil inmutable y resultado
  correlacionado; Unsupported no instala. Ejecutar antes del ensamblaje final y
  confirmar el fallo que falta resolver.
- [ ] **2. Integrar** CLI/proveedores/jobs/UI y resolver fallos del flujo, sin
  saltarse políticas ni convertir errores en éxito para completar la prueba.
- [ ] **3. Ejecutar** `scripts/verify.sh`: cargo fmt, cargo test, clippy, validación
  de manifiesto, qmllint y QtTest deben pasar. `git diff --check` debe quedar vacío.
- [ ] **4. Instalar y validar** el conjunto preparado en Korax; backup previo de
  shell.json y componentes propios; comprobar plugin enabled, configerrors y
  logs nuevos. Abrir/cerrar por clic, Escape e IPC; probar búsqueda, selección,
  Check, detalle, plan y rechazo. Capturar únicamente el panel con datos de apps
  para aceptación visual, sin conversaciones ni perfiles. Probar disable/enable
  y restart shell; restaurar widget si aparece regresión.
- [ ] **5. Verificar** recibos y estado por app. Actualización real solo si hay
  versión nueva y se acepta su plan gráfico; nunca degradar para producir prueba.
  Si no hay pendiente, dejar esa prueba marcada como no ejercitada y conservar
  pruebas del ejecutor simulado. Confirmar destinos IA sin consultas ni cambios
  por el plugin, perfiles conservados y ausencia de cierres automáticos.
- [ ] **6. Solicitar** revisión independiente del cambio completo usando el método
  aprobado y el spec/plan como referencia; resolver hallazgos y repetir solo los
  checks afectados. Integrar en main con commits enfocados tras validación.
- [ ] **7. Commit:** `test: verify app update workflows and Omarchy installation`.

## Cobertura y revisión del plan

| Sección de la especificación | Tareas |
| --- | --- |
| 1 Resultado y alcance | T3, T4, T14, T16 |
| 2 Integración y responsabilidades | T1, T2, T13, T14 |
| 3 Inventario y selección | T3, T4 |
| 4 Exclusión IA | T4, T8, T9, T16 |
| 5 Proveedores | T5, T6, T7, T10, T11, T12 |
| 6 Versiones y estado | T1, T2, T5, T6, T7, T12, T14 |
| 7 Interacción y comprobaciones | T1, T13, T14 |
| 8 Plan y transacciones | T8, T9, T10 |
| 9 Privilegios y persistencia | T9, T13, T15 |
| 10 Datos y procesos | T6, T11, T12, T15, T16 |
| 11 Estado y contrato | T1, T4, T13 |
| 12 Instalación y mantenimiento | T13, T14, T15, T16 |
| 13 Aceptación | T16 y pruebas de cada tarea |
| 14 Referencias y revisión | Fuentes siguientes y revisión de este plan |

Revisión propia: interfaces y tipos comunes consistentes, cinco casos de Review
Focus asignados a pruebas, cobertura de las 14 secciones, pasos con resultados
observables y sin cuerpos de implementación anticipados. Los nuevos nombres de
tipos auxiliares se definen en la tarea que los produce; los tests se escriben
antes del código correspondiente. No se omite un proveedor para dar el producto
por completo; toda limitación real queda en UI, recibo e informe final.

## Referencias verificadas para ejecutar

- [Guía de plugins](https://plugins.omarchy.org/develop.html), contrastada con
  `/usr/share/omarchy/shell/README.md` y el plugin Rust local de MINSAL.
- [alpm.rs y compatibilidad](https://docs.rs/alpm/latest/alpm/): la documentación
  consultada de 5.0.2 declara libalpm 15; Korax tiene 16.0.1. T2 resuelve esa ABI.
- [bindgen](https://docs.rs/bindgen/latest/bindgen/) y `/usr/include/alpm.h` instalado.
- [pacman-conf](https://man.archlinux.org/man/pacman-conf.8.en) para configuración
  efectiva, includes y repositorios.
- [pkexec](https://man.archlinux.org/man/pkexec.1.en) para autorización nativa,
  identificación del caller y desactivación del agente textual interno.
- `omarchy-update-lock` instalado usa flock sobre
  `${XDG_RUNTIME_DIR}/omarchy-update.lock`; conservar el mismo bloqueo de sesión.

Estado: plan preparado para revisión; ejecución pendiente. Este documento no
afirma que los tests, binarios, helper ni plugin estén implementados o instalados.
