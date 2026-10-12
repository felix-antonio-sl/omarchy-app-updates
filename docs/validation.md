# Validación de Apps en Korax

El motor Rust, el panel nativo QML, la preparación gráfica y el ejecutor
administrativo están implementados. La aceptación se realiza en Korax con
Omarchy 4.0.4, Qt 6.11.2, Rust 1.99.0 y libalpm 16.0.1, el 12 de octubre de 2026.
Los recibos y capturas del host permanecen privados fuera de Git.

## Verificación del código

`scripts/verify.sh` pasó con 124 pruebas de Rust y 13 entradas QtTest, cero
fallos, Clippy con `-D warnings`, formato, validación del manifiesto y `git diff
--check`. El lint QML utiliza los metadatos reales instalados; sus advertencias
restantes son propiedades QObject de Style y QProcess::ExitStatus, reproducidas
en componentes del shell instalado. No se modificaron módulos empaquetados.

Los tests usan rutas temporales y proveedores simulados. El flujo de aceptación
incluye descubrimiento real de lanzadores de prueba, exclusiones, selección JSON,
consulta, plan, reserva del trabajador, revisión de artefacto con digest nuevo,
canal privado de decisiones, extracción nativa y activación de Telegram de prueba,
verificación del resultado y proyección del estado. Un destino Unsupported impide
instalar; una decisión del alcance anterior no activa archivos. Los perfiles de
prueba quedan idénticos.

También se ejecutaron explícitamente tres pruebas opcionales: construcción e
inspección de un paquete local mediante makepkg, y dos unidades transitorias de
systemd del usuario. Una unidad mantuvo el ejecutable fijado después del cambio
de fuente y de desconectar la observación; el coordinador real completó un plan
vacío aislado con un recibo nuevo. La prueba ALPM instala un paquete de prueba en
una raíz temporal, sin modificar paquetes del host. Estas pruebas no equivalen a
haber actualizado aplicaciones reales.

## Revisión independiente y correcciones

Un revisor gpt-6-astra examinó el cambio completo 45fc5e5..3bf571d sin modificar
el checkout, ejecutar Cargo ni elevar privilegios. No estableció hallazgos
críticos; cinco hallazgos importantes se corrigieron en una pasada, cada uno
con una regresión que falló antes de la corrección y pasó después:

- Flatpak: preparar las apps seleccionadas juntas por instalación y compartir
  una sola transacción de sus runtimes.
- Bootstrap: finalizar una preparación observada después de recargar el shell,
  conservando la correlación de una solicitud nueva frente a recibos anteriores.
- Inventario: conservar instalaciones Flatpak de usuario y sistema con el mismo
  nombre de archivo de lanzador y ramas distintas.
- AUR: agrupar los destinos de una base de paquetes divididos y construirla una
  sola vez; adjuntar todos los artefactos al mismo alcance.
- Telegram: inscripción explícita por GUI, descargando una publicación oficial
  fija y comparando ambos ejecutables antes de asociar una versión. Si no
  coinciden, permanece sin verificar y no se propone instalar o degradar.

Se completaron además el baseline privado de recetas AUR instaladas, el filtro
Problemas y la presentación de autorizaciones y tamaños conocidos del plan.
La aceptación detectó que Cargo crea hardlinks entre release y deps: el
preparador ahora fija copias privadas independientes antes de inspeccionar y
empaquetar. Su regresión también pasó tras fallar inicialmente.

## Comprobación real de versiones

La consulta en Korax detectó 58 apps, 59 componentes y 22 apps seleccionadas.
La cobertura de updater.py quedó verificada. Sus siete destinos no aparecen
ni se consultan como apps administradas. No se ejecutó updater.py.

Ghostty, Chrome, Protégé, Remmina, GeForce NOW, Steam, TickTick, VS Code y Voxtype
coincidieron con sus canales. WhatsApp y los otros accesos web conservan su
estado de servicio web. Audacity tiene una publicación del fabricante 4.0.1
posterior al paquete 3.7.9 del canal. Thunderbird 157.0.1 está por delante del
repositorio consultado, que devolvió 155.0; Apps no propone degradarlo. El
runtime de GeForce NOW está fuera de soporte y esa información aparece en sus
detalles; la migración corresponde a una publicación del proveedor.

El archivo histórico de Telegram no coincidió con el checksum de su recibo,
así que no se usó para inferir una versión. La nueva acción de inscripción
obtuvo el release oficial fijo 7.3.1, comprobó ELF/bibliotecas y la igualdad de
Telegram y Updater con los archivos ya instalados. Escribió únicamente el
metadato privado asociado al hash; Telegram quedó verificado y al día, sin
abrirlo ni reemplazarlo. En otra instalación sin metadatos, Comprobar → Detalles
→ Verificar binario de Telegram reproduce esta operación. Una versión antigua
que no coincida con la publicación consultada requiere revisar su procedencia.

## Instalación y aceptación nativa

El paquete `omarchy-app-updates-helper` 0.1.0-1 quedó instalado mediante el
diálogo nativo de autorización. Helper, política y licencia pertenecen a root;
`pacman -Qkk` comprobó 11 archivos y cero alteraciones. El motor y el helper
comparten protocolo 1, versión 0.1.0, digest de fuentes y ABI 16.0.1.
El widget quedó habilitado bajo `~/.config/omarchy/plugins/korvo.app-updates/`.

La aceptación comprobó apertura por clic e IPC, Escape con la API de teclado
documentada de Hyprland, búsqueda de WhatsApp, detalle del servicio web,
exclusión y reincorporación de esa app conservando las 22 seleccionadas,
comprobación forzada a través del controlador del panel, disable/enable y
reinicio del shell. El resto de `shell.json` coincide exactamente con el
respaldo previo al quitar únicamente la entrada de Apps. Hyprland no reporta
configerrors. Las capturas conservadas muestran solo el contenido del panel.
Una captura de la animación inicial se descartó y se repitió sobre un escritorio
vacío, esperando el final de la transición.

La primera preparación se detuvo antes de autenticar por los hardlinks de
Cargo. Una activación posterior encontró que rescanPlugins retorna antes de
terminar el catálogo; se corrigió la espera y la instalación completa posterior
produjo un recibo exitoso. La consulta al abrir conservó la fecha de la caché:
los destinos excluidos se filtran antes de reconciliar el inventario. Las
regresiones de ambas situaciones fallaron antes de la corrección y pasaron
junto a la suite completa.

La prueba visual también encontró que DelegateModel representa las listas
anidadas mediante secuencias Qt: Array.isArray devolvía false y ocultaba las
versiones en las filas. El adaptador ahora acepta esas secuencias; una prueba
comprueba el texto y la altura del estado realmente renderizado. Steam distingue
su lanzador y cliente mediante el tipo del componente, con el ID real del
inventario. Estas correcciones pasaron QtTest y la validación nativa.

La comprobación final detectó cero actualizaciones instalables para las 22 apps
seleccionadas. Una actualización real de app, su autorización final y el rechazo
de un alcance pendiente en la GUI quedaron **no ejercitados** por falta de una
versión pendiente. No se fabricó una actualización ni se degradó un paquete.
Los planes, decisiones y ejecución se verificaron con fixtures significativos,
QtTest y el ejecutor nativo en una raíz temporal. El plan vacío de la consulta
real también se puede preparar sin instalar aplicaciones.

Los datos completos, logs, respaldos y capturas se guardan en
`~/.local/state/omarchy-app-updates/acceptance/`; no se incluyen perfiles,
credenciales ni inventarios detallados en este repositorio.

## Decisiones de implementación

- Ruling: Native worktree creation failed because the task cwd is not a git repository — use the ignored Git worktree for the requested repo — cost if wrong: duplicate worktree registration; native operation failed with no paths.
- Ruling: Plan task headings Tn are not recognized by task-start — normalize to Task n without changing task content — cost if wrong: bookkeeping only.
- Task 6: Ruling: Installed Flatpak CLI has no assumeno/dry-run update — use libflatpak ready-pre-auth to prepare operations and reject before deploy, then compare the exact operation scope before commit — cost if wrong: metadata preparation could require a retry; native host test must prove no deployment.
- Task 7: Ruling: Ghostty releases/latest returned 404; use stable semantic version tags from its official GitHub repository, verified against ghostty.org/download — cost if wrong: publication status may need source review; package installation still uses the configured repository.
- Task 9: Ruling: Installed libalpm headers declare a combined sandbox setter absent from the shared library exports — preserve DisableSandbox through the exported filesystem and syscall component setters, verified against the live ABI — cost if wrong: a future sandbox component requires preparation review rather than silent policy changes.
- Task 9: Ruling: No ejecutar migraciones Omarchy nuevas desde el helper sin revisión de compatibilidad — el límite de updater.py exige conocer sus efectos, y no existe una lista aprobada de IDs/digests; se bloquea todo el alcance si un paquete aporta scripts de migración — coste si es demasiado restrictivo: una actualización de Omarchy requerirá ampliar la lista revisada antes de usar Apps.
- Task 9: Ruling: La instalación normal necesita el bloqueo propio de libalpm, como confirmó una prueba real en root temporal — mantener el bloqueo de preparación hasta Confirm, relevarlo por alpm_trans_init sin NOLOCK y volver a validar antes de promover bases; un competidor provoca Busy sin promoción — coste si la ventana de relevo encuentra pacman activo: el usuario debe preparar de nuevo, sin saltarse la transacción ajena.
- Task 12: Ruling: El cliente instalado contiene -forcesteamupdate y -silent, pero no se confirmó -update/-quit — iniciar el actualizador oficial en una unidad propia con los flags comprobados, sin cierre ni señales a juegos, y verificar manifiesto más inventario instalado y CRC de binarios — coste si Valve cambia esos flags: Apps detiene la acción y requiere usar el lanzador o revisar el adaptador; Steam puede conservar su ventana al terminar.
- Task 14: Ruling: Installed Quickshell modules are statically registered in quickshell, so standalone qmltestrunner cannot load quickshell-coreplugin; qs.* is also a dynamic shell import root — run QtTest with explicit no-I/O Quickshell doubles and copies of the real installed Ui/Commons modules in a private import bridge, then validate the actual widget inside the live Omarchy process and lint against real installed metadata — cost if wrong: unit tests cannot prove compositor focus or IPC; live acceptance must do that separately. No packaged module is modified and no native process starts during QtTest.
- Task 14: Ruling: Generic reopen can terminate the active Voxtype daemon while launching only its configuration UI — disable that reopen button and retain a pending-version explanation until the operator finishes dictation and uses Voxtype configuration — cost: Voxtype activation is manual instead of the generic one-click reopen flow; no daemon is closed automatically. Other apps require a separate explicit close-and-reopen action.
- Ruling: Run the one fresh final code review while release compilation and live acceptance proceed, before privileged deployment — code is assembled and the security boundary benefits from review before install; review receives live evidence later without an additional reviewer — cost if wrong: a live-only regression is resolved in the same final fix pass.
- Final: Ruling: Host deployment and real GUI behavior cannot be inferred from static review — verify separately on Korax; real app update remains unexercised if no pending version or accepted plan — cost if wrong: a compositor/auth regression may remain until actual operation; report exact evidence and limitations.
- Final: Ruling: Approved AUR recipes run as the user without a general user-files sandbox — this is the approved reviewed-recipe model; preserve privilege separation and pinned recipe/artifact checks — cost if wrong: a malicious approved recipe can affect user files beyond its packaging outputs.
- Final: Ruling: Unreviewed Omarchy migrations remain blocked — their effects on excluded AI tools cannot be established by the package list alone — cost if wrong: Omarchy updates require compatibility review before Apps can apply them.
- Final: Ruling: Cargo discovery targets the installed Korax stable toolchain — the plugin's current host prerequisite is explicit and verified — cost if wrong: another host/toolchain layout requires adapting preparation.
