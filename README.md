# Apps para Omarchy

Un widget nativo de Omarchy Quattro para ver las versiones instaladas, las
actualizaciones disponibles en tu canal y las publicaciones del fabricante.
El motor está escrito en Rust; la interfaz utiliza QML y los componentes del
shell. La lista de aplicaciones es editable y conserva tu selección.

Instalado y validado en Korax el 12 de octubre de 2026. El motor Rust y el
ejecutor administrativo comparten versión, protocolo, fuentes y ABI verificadas.
La [evidencia de validación](docs/validation.md) describe los tests y las pruebas
del panel dentro del shell real.

Abre **Apps** en la barra y usa **Comprobar** para consultar las fuentes. El
contador agrupa aplicaciones, aunque tengan varios componentes, como el
lanzador de Steam y su cliente. **Detalles** separa instalada, canal y fabricante;
una publicación nueva del fabricante no significa que tu canal ya la distribuya.
Las apps web muestran que el servicio se actualiza automáticamente.

Usa el círculo de cada fila para incluirla o excluirla. **Revisar plan** muestra
el alcance antes de iniciar una actualización. Arch requiere una actualización
completa y coherente: también aparecen los paquetes de sistema y dependencias
necesarios. Las recetas AUR muestran su commit, archivos, cambios y hashes; se
compilan como usuario. Los artefactos y el alcance administrativo final requieren
una confirmación nueva cuando cambian.

El ejecutor pide autorización mediante el diálogo del sistema. La instalación
confirmada continúa al cerrar el panel o recargar el shell. **Trabajo** e
**Historial** muestran resultados por componente. Reabrir una aplicación exige
una acción separada; guarda tus cambios antes de solicitar el cierre. Voxtype
permanece activo y su activación se deja para cuando termines el dictado. Escape
cierra el panel; Tab recorre sus controles y Ctrl+Tab cambia al panel vecino.

Los siete destinos gestionados por `~/.local/share/ai-updaters/updater.py` se
excluyen de inventario, consultas, contador y planes. Si cambia esa cobertura,
Apps bloquea la instalación hasta revisar la política. También bloquea un
alcance de Arch que los incluya; no crea una actualización parcial. Las
migraciones nuevas de Omarchy requieren revisión previa de compatibilidad.

**Preparar motor** compila las fuentes y dependencias fijadas con Cargo.lock,
con dos procesos y caché fuera del plugin. Puede reconstruir el motor aunque
una nueva ABI impida arrancar el binario anterior. El instalador prepara un
paquete local `omarchy-app-updates-helper`, revisa sus archivos y dependencias e
instala exclusivamente ese paquete mediante autorización gráfica. El ejecutor
queda en `/usr/lib/omarchy-app-updates/app-updates-helper`, con propiedad de root
y política `auth_admin`; no se concede acceso sin contraseña. No se compila AUR
ni Cargo como root y no se sincronizan paquetes parcialmente para preparar Apps.

Añadir o clonar un repositorio mediante Omarchy solo descarga y valida el plugin;
no compila Rust ni instala el ejecutor. La preparación es una acción explícita.
Los requisitos de compilación son Rust estable, Cargo, Clang, pkg-config y los
headers de libalpm, libflatpak y libarchive ya instalados. La preparación evita
reemplazar el plugin mientras haya trabajos activos. Los binarios producidos por
Cargo se fijan como copias privadas independientes antes de validarlos; la
activación espera a que el catálogo asíncrono de Omarchy registre el plugin.

Si una instalación local de Telegram carece de metadatos, usa Comprobar →
Detalles → Verificar binario de Telegram. La acción descarga la publicación
oficial fija y compara Telegram y Updater con los archivos instalados, sin
abrir ni reemplazar la app. Solo asocia una versión cuando ambos coinciden;
si no coinciden, informa que debe revisarse la procedencia.

La instalación prepara un conjunto completo de archivos en staging, conserva
la versión anterior y respalda `shell.json`; la activación usa el gestor de
plugins de Omarchy. Las aplicaciones administradas y sus perfiles se conservan
al retirar Apps. La retirada completa del ejecutor utiliza su propio paquete;
desactivar el widget desde Omarchy mantiene ese paquete para poder reactivarlo.

El desarrollo local vive en `/home/korvo/Projects/omarchy-app-updates` y el plugin
se instala en `~/.config/omarchy/plugins/korvo.app-updates/`. Configuración, caché,
recibos y registros privados están en `~/.config/omarchy-app-updates/` y
`~/.local/state/omarchy-app-updates/`. Los inventarios y datos del host no se
incluyen en el repositorio.

Para desarrollo, `cargo test`, `cargo clippy --all-targets -- -D warnings`,
`tools/test-qml`, `tools/lint-qml` y `omarchy plugin validate .` verifican el
motor y el panel. QtTest utiliza los componentes instalados de Omarchy con
sustitutos de Quickshell que no hacen I/O; foco, IPC y ciclo de vida se comprueban
por separado dentro del shell real. Las pruebas de paquetes y proveedores usan
rutas temporales y no actualizan las aplicaciones del host.

El contrato sigue la [guía de desarrollo de plugins](https://plugins.omarchy.org/develop.html).
La [especificación](docs/superpowers/specs/2026-10-11-app-updates-design.md) y el
[plan](docs/superpowers/plans/2026-10-11-app-updates.md) registran el alcance aprobado.
Licencia GPL-3.0-or-later, incluida en LICENSE.
