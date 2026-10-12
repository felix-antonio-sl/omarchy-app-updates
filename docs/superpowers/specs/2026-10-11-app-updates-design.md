# Apps: estado y actualización de aplicaciones de escritorio en Omarchy

Fecha: 2026-10-11. Estado: especificación aprobada por el usuario al solicitar
el plan de implementación. Destino inicial: Korax, Omarchy 4.0.4-1.

## 1. Resultado esperado y decisiones aprobadas

El usuario quiere comprobar el estado de actualización de sus aplicaciones,
conocer versiones nuevas y actualizar desde un plugin integrado en Omarchy.
El motor se implementa en Rust. Se utiliza como referencia el catálogo de
plugins de Omarchy y se excluye todo lo que administra el actualizador de IA.

El usuario eligió aplicaciones de escritorio con una lista editable. La lista
inicial se obtiene del inventario y se verifica contra la instalación real;
no se considera que todo paquete explícito haya sido instalado por el usuario.
Los componentes base de Omarchy quedan fuera de la selección predeterminada.
El usuario puede incluir aplicaciones base desde el panel cuando así lo desee.
La exclusión del actualizador de IA tiene precedencia sobre la selección.

El nombre visible es **Apps** y el identificador es `korvo.app-updates`.
El repositorio vive en `/home/korvo/Projects/omarchy-app-updates` en Korax.
La publicación en GitHub o en el catálogo no forma parte de esta especificación.

## 2. Integración nativa y responsabilidades

La interfaz usa los componentes compartidos de la versión instalada de Omarchy,
sus colores, tipografía, escalado y navegación de paneles. El manifiesto raíz
declara `bar-widget` y referencia `ui/BarWidget.qml`; el widget carga
`ui/Panel.qml` internamente. No se declara un segundo tipo para ese panel.

Rust contiene descubrimiento, identidad de aplicaciones, exclusiones,
comparación de versiones, proveedores, planificación, ejecución y persistencia.
QML presenta el estado y envía solicitudes estructuradas al motor. No construye
comandos de instalación ni recibe credenciales administrativas.

Las unidades principales son:

- **Inventario:** relaciona lanzadores con paquetes, referencias Flatpak,
  instalaciones locales o sitios web, y agrupa lanzadores de una misma app.
- **Política:** aplica exclusiones obligatorias y selección del usuario.
- **Proveedores:** descubren versiones y describen acciones posibles.
- **Coordinador:** prepara planes, gestiona bloqueos y trabajos persistentes.
- **Ejecutor administrativo:** realiza las operaciones de paquetes autorizadas.
- **Panel:** lista, detalles, selección, comprobación, actualización e historial.

El proceso que comprueba versiones termina al finalizar su trabajo. No hay un
servidor HTTP ni un proceso Rust permanente solo para mantener el indicador.
Las operaciones largas viven en unidades systemd de usuario y sobreviven al
cierre del panel y a la recarga de Quickshell.

## 3. Descubrimiento y selección

Se recorren los directorios de aplicaciones XDG de usuario y sistema, incluidas
las exportaciones Flatpak. Se respetan precedencia, `Hidden` y `NoDisplay`.
Los archivos `.desktop` se leen como datos: descubrir una app nunca ejecuta
su campo `Exec`, un instalador, una app ni un wrapper de descarga bajo demanda.
El campo `Version` de un `.desktop` no representa la versión de la aplicación.

La identidad incluye el proveedor y su destino: nombre de paquete, referencia
Flatpak con instalación y rama, instalación local reconocida o URL web.
Dos instalaciones de una misma aplicación se muestran por separado. Varios
lanzadores del mismo paquete se agrupan, conservando sus nombres secundarios.
Se verifica la propiedad del ejecutable y de los lanzadores cuando corresponda;
no se atribuye una app al paquete de un intérprete o wrapper genérico.

El inventario previo es una semilla, no una fuente de estado permanente.
La clasificación inicial compara con los listados base instalados de Omarchy
y con los datos de pacman. Esa clasificación se presenta como una sugerencia,
sin afirmar quién instaló históricamente un paquete.

El panel **Gestionar aplicaciones** permite buscar, incluir y excluir apps.
Una app nueva se ofrece para incorporar; no se actualiza por aparecer en el
inventario. Una app eliminada deja de contar y se conserva solo en el historial.

## 4. Exclusión del actualizador de IA

Se excluyen los destinos actuales de `updater.py`: `codex`, `claude`,
`antigravity`, `opencode`, `hermes`, `claude-desktop` y `chatgpt`.
La exclusión abarca sus alias, lanzadores e instalaciones correspondientes,
incluidos los nombres visibles Claude y ChatGPT/Codex Desktop.

La comprobación de esta cobertura es de lectura. No se importa ni ejecuta el
módulo Python; no se invoca `status`, `check`, instaladores ni comandos de esas
herramientas. El motor verifica la declaración literal de destinos de la suite
local con un lector limitado y compara con su mapa de identidades conocido.
Si cambia el formato o aparece un destino sin mapa, se informa que debe revisarse
la cobertura y se bloquean las actualizaciones hasta reconciliarla.

Las aplicaciones excluidas no aparecen en la lista gestionada ni en el contador.
La configuración no permite saltarse esta exclusión. No se reemplazan sus
directorios, no se ejecuta `mise up` y no se llama a `omarchy update` completo.
Las dependencias generales del sistema siguen formando parte de una transacción
Arch completa, pero no son destinos gestionados por el plugin.

El paquete `hermes-desktop` es una aplicación distinta del checkout Hermes CLI.
Se descubre como candidato independiente y queda sin seleccionar inicialmente;
su inclusión posterior no autoriza modificar el CLI ni ejecutar su actualizador.

## 5. Proveedores y cobertura

| Proveedor | Comprobación | Actualización |
| --- | --- | --- |
| Repositorios Arch, incluido Omarchy | Base temporal sincronizada, orden real de repositorios y comparación de versiones de pacman | Transacción completa de paquetes, con alcance visible y autorización gráfica |
| AUR sin paquete en repositorios | RPC de AUR, metadatos y receta correspondiente al paquete instalado | Revisión gráfica de la receta y sus cambios, compilación como usuario e instalación del paquete construido |
| Flatpak | Instalación de usuario o sistema, remoto, rama, versión y commit | Referencias seleccionadas y sus runtimes necesarios mediante Flatpak |
| Telegram local reconocido | Distribución oficial y metadatos del binario, evitando abrir la app para consultar | Preparación, verificación y cambio de instalación conservando sus datos |
| Cliente Steam | Manifiesto local y manifiesto oficial del canal instalado | Actualizador oficial; no incluye juegos ni cambios de canal |
| Acceso web | URL y navegador efectivo del lanzador | El sitio se actualiza al cargarlo; se gestiona el navegador una sola vez |
| Instalación desconocida | Identidad y método detectados, sin ejecutar su lanzador | Sin proveedor; acción deshabilitada y explicación |

El conjunto inicial reconocido incluye Audacity, GeForce NOW, Ghostty,
Google Chrome, Protégé, Remmina, Steam, Telegram, Thunderbird, TickTick,
Visual Studio Code y Voxtype. También se muestran WhatsApp y los demás accesos
web seleccionados con su naturaleza web. Orca, que fue desinstalado, no se incluye.

Los paquetes auxiliares necesarios se asocian a la app: por ejemplo, el idioma
instalado de Thunderbird se mantiene compatible, sin cambiar el idioma elegido.
Voxtype conserva la variante de CPU y la configuración de motor instalada.

Cuando un paquete existe en un repositorio configurado, se respeta ese proveedor
antes de proponer compilarlo desde AUR. Las recetas AUR no se ejecutan por buscar
versiones. Se fija el commit revisado; un cambio exige revisar el plan de nuevo.
Una compilación fallida no se presenta como instalación ni dispara un reintento
automático de scripts. Las dependencias nuevas aparecen en el alcance aprobado.

## 6. Versiones y significado del estado

Cada fila muestra instalada, disponible en su canal y última comprobación.
Los detalles muestran fuente, canal y nueva publicación oficial cuando hay un
adaptador de fabricante. La cobertura de publicaciones se declara por app;
no se promete identificar la última versión mundial de cualquier instalación.

Se conservan versiones originales junto a su comparación normalizada. Los
paquetes usan las reglas de pacman, incluidos epoch y revisión de empaquetado.
La publicación del fabricante se compara con la versión de producto, no con
una cadena de paquete completa. Se utilizan commits para Flatpak si no existe
una versión útil. Las versiones de canales diferentes no se mezclan.

Los estados de comprobación son:

- **Al día:** una consulta vigente confirma coincidencia con el canal.
- **Actualización disponible:** existe una versión instalable más reciente.
- **Nueva versión aún no disponible en tu canal:** publicación oficial posterior
  a la instalable, sin cambiar de proveedor para instalarla.
- **Versión instalada posterior al canal:** no se propone una degradación.
- **No se pudo comprobar:** error, fuente inaccesible o comparación imposible.
- **Sin proveedor:** falta un adaptador para esa instalación.
- **Web: actualización automática:** depende del sitio y del navegador.

Separadamente se muestran las fases de operación: preparada, en cola,
descargando, compilando, esperando autenticación, instalando, verificando,
completada, fallida, cancelada y **Reabrir para aplicar**.

El contador de barra solo suma destinos seleccionados con actualización
instalable verificada. Un paquete y sus lanzadores cuentan una vez. Un cliente
Steam y su paquete son componentes de una app; sus detalles permanecen separados.
Los accesos web, fallos de consulta y publicaciones sin paquete no inflan el
contador; los errores tienen un indicador y una explicación propios.

## 7. Interacción y comprobaciones

Un clic abre o cierra el panel; Escape lo cierra y la navegación respeta los
controles del shell. El panel adapta su tamaño a la pantalla y permite desplazar
la lista. No exige un atajo nuevo para utilizarlo.

El usuario puede filtrar todas, con actualizaciones y con problemas; buscar por
nombre; abrir detalles y fuentes; seleccionar destinos; comprobar; actualizar
seleccionadas; gestionar la lista; y consultar el historial por operación.

Al abrir se muestra inmediatamente la caché con su fecha. Si tiene más de seis
horas, se inicia una comprobación. **Comprobar** fuerza una consulta nueva.
Por defecto no se instala nada automáticamente ni se configura un temporizador
permanente. Las consultas de red se limitan a dos proveedores en paralelo y
tienen tiempos de espera acotados; se publica el resultado de cada proveedor.

Un fallo mantiene el dato anterior marcado como antiguo y registra el error;
no lo convierte en «Al día». Se reintentan solo consultas y descargas transitorias,
no una instalación o una migración. Las acciones necesitan un plan nuevo aunque
la fila ya contenga un resultado en caché.

## 8. Planificación y transacciones Arch

**Actualizar seleccionadas** prepara un plan sin instalar. El plan enumera
apps, componentes, cambios de versión, dependencias, paquetes del sistema,
descarga prevista cuando se conoce, necesidad de autenticación y aplicaciones
que requerirán reabrirse. No selecciona por defecto destinos desconocidos.

Para Arch se ofrece una actualización completa de paquetes con vista de todos
los cambios, incluidos los no seleccionados que exige mantener el sistema
coherente. Excluir una app del inventario no significa fijar su paquete dentro
de una transacción completa. Esta diferencia se explica en el plan, antes de
autorizar. Rechazar ese alcance deja la operación Arch sin ejecutar; otras
actualizaciones independientes pueden realizarse con su propio plan.

Las consultas usan bases temporales, nunca una sincronización parcial de la
base del sistema. El trabajador adquiere los bloqueos del plugin y coopera con
el bloqueo de actualización de Omarchy y el de pacman. Una operación externa
concurrente provoca espera o rechazo con explicación, sin borrar su bloqueo.

La fase administrativa revalida el estado y el alcance antes de confirmar la
transacción. Si han cambiado las versiones, dependencias o decisiones, devuelve
un plan revisado y espera aceptación gráfica. No instala paquetes adicionales
por una aprobación de un plan anterior. El contrato permite preparar y revisar
la transacción mediante libalpm sin responder ciegamente a conflictos.

Se utiliza el mecanismo de snapshot disponible en Omarchy y se informa si no
está disponible o falla. No se promete una restauración automática del sistema.
Si cambia el paquete Omarchy, sus migraciones necesarias se incluyen y revisan
en el plan. No se ejecutan la etapa global de mise, limpieza de huérfanos ni
hooks generales de actualización que puedan tocar la suite excluida.
Una migración incompatible con ese límite requiere intervención explícita y
detiene el flujo automático, conservando el detalle de lo ya realizado.

La instalación AUR se realiza después de asegurar coherencia de los paquetes
del sistema. Una actualización de sistema completada y una compilación AUR
fallida se registran como resultados separados; no se anuncian como revertidas.

## 9. Privilegios y ejecución persistente

El motor normal corre como usuario. Las solicitudes contienen IDs y opciones
tipadas, nunca comandos de shell. Las consultas no requieren autenticación.

Un ejecutor Rust administrativo, instalado con propiedad de root, expone solo
operaciones de paquetes previstas. Se autoriza mediante polkit con diálogo
gráfico. Rechaza binarios auxiliares modificables por el usuario, rutas externas,
solicitudes desconocidas y respuestas que no correspondan al trabajo activo.
No acepta un ejecutable arbitrario ni una línea de comandos enviada por QML.

En AUR, compilación y lectura de recetas permanecen como usuario. El ejecutor
administrativo copia y verifica los artefactos aprobados en staging propio antes
de instalarlos, evitando enlaces y sustituciones entre revisión e instalación.

Cada trabajo tiene un identificador único, unidad systemd, plan y registro.
El trabajador conserva el ejecutable de su versión durante la operación para
que una actualización del propio plugin no reemplace su código a mitad del
trabajo. Un segundo panel puede observarlo, pero no inicia otra instalación
concurrente. No se cancela por cerrar la interfaz.

La cancelación detiene etapas preparatorias cuando es seguro. Durante una
transacción de paquetes ya confirmada, se permite cerrar el panel y seguir
observando, pero no ofrecer una interrupción que pueda dejarla incompleta.

## 10. Datos personales y procesos abiertos

El plugin no solicita ni almacena credenciales de las aplicaciones. Las
contraseñas administrativas se introducen en el agente polkit y no pasan por
el motor, argumentos de procesos, historial ni registros.

Los perfiles, cuentas y proyectos quedan fuera de los directorios reemplazados.
Telegram mantiene su directorio de datos y conserva la instalación previa para
restaurar el binario si la activación falla. Las descargas requieren HTTPS,
origen permitido, validación del contenido y checksum o firma cuando el
proveedor publique una referencia verificable. No se presenta un hash calculado
localmente como prueba de autenticidad del fabricante.

Una actualización de paquete puede completar la instalación sin cerrar una app.
Si sigue ejecutando el binario anterior, se muestra **Reabrir para aplicar**.
Los reinicios requieren una acción explícita en el panel. No se fuerza el cierre
de sesiones Remmina, editores, correo, navegador, dictado ni juegos.

Para cambiar una instalación local activa se solicita cerrar la app o aceptar
su reapertura. Para actualizar el cliente Steam se espera a que no haya juegos
ni operaciones incompatibles en curso. Si no se puede confirmar, se espera a
que el usuario cierre Steam. La actualización de su paquete queda separada.

## 11. Persistencia y contrato con la interfaz

La configuración vive en `~/.config/omarchy-app-updates/config.json` e incluye
selección, preferencias de presentación y versión del esquema. Se escribe de
forma atómica y se conserva una copia anterior. La exclusión de IA no puede
desactivarse mediante este archivo.

La caché, resultados y trabajos viven en `~/.local/state/omarchy-app-updates/`,
con permisos privados. No se suben al repositorio inventarios personales,
perfiles, archivos de configuración reales ni registros de uso del host.

La interfaz llama a un cliente Rust por argumentos fijos. Las operaciones
previstas son `inventory`, `status`, `check`, `plan`, `apply`, `selection`,
`history` y `operation`. Los datos variables viajan como JSON por stdin.
Las respuestas incluyen versión de protocolo, ID de solicitud, resultado o
error estructurado. Los eventos se correlacionan con el ID de operación.

El reporte separa identidad de app, comprobación, plan y operación. Una respuesta
`accepted` solo confirma que un trabajo fue recibido. La finalización requiere
el resultado de ese trabajo y la verificación del proveedor; un resultado
histórico no confirma una ejecución nueva. Tras una caída, se comprueba la unidad
y el estado instalado antes de declarar finalización o permitir un reintento.

## 12. Instalación y mantenimiento del plugin

El repositorio tendrá manifiesto, QML, fuentes Rust, lockfile, licencia y guía
de instalación. El motor y el ejecutor se compilan con versiones coherentes.
No se mantiene un enlace simbólico dentro del directorio instalado del plugin.

El instalador de plugins de Omarchy solo clona y valida; no compila Rust ni
ejecuta hooks. Se documenta esta separación. La instalación inicial en Korax
prepara los binarios e instala el ejecutor mediante autenticación gráfica.
El panel detecta un motor ausente o incompatible y ofrece la preparación gráfica
correspondiente, sin suponer que añadir el repositorio preparó el motor.

La compilación solo usa fuentes y dependencias fijadas del repositorio elegido;
no se recompila al abrir cada panel. El diálogo de preparación indica qué se
descargará y muestra progreso. Una versión incompatible no puede actualizar apps
hasta completar esa preparación. Las operaciones activas conservan su versión.

Los archivos vivos se preparan en staging y se reemplazan como un conjunto. Se
respalda `shell.json`, se añade el widget sin alterar los existentes y se valida
la configuración. Se trabaja en las rutas del usuario; los archivos empaquetados
de `/usr/share/omarchy` se utilizan como referencia, sin editarlos.

La desinstalación desactiva el widget y retira los componentes propios cuando
no hay trabajos activos. No desinstala las apps administradas ni borra sus datos.

## 13. Verificación y criterios de aceptación

La implementación debe demostrar:

1. Inventario real, precedencia XDG, agrupación, instalaciones múltiples,
   eliminación de Orca y ausencia de ejecución de lanzadores durante consultas.
2. Exclusión de los siete destinos de IA en inventario, consulta, contador,
   planes y ejecución; detección de cambios en la cobertura de `updater.py`.
3. Comparación correcta de versiones Arch, producto, canal y commits Flatpak;
   tratamiento explícito de fuente caída y versión instalada posterior.
4. Comprobaciones sin modificar la base de pacman del sistema ni instalar.
5. Plan con alcance completo, rechazo de planes antiguos y de solicitudes
   arbitrarias; autenticación gráfica y compilación AUR sin root.
6. Revisión AUR fijada a su commit y rechazo de artefactos sustituidos o inseguros.
7. Trabajo que sobrevive al cierre del panel y a una recarga del shell, con
   bloqueos y resultados asociados a la operación correcta.
8. Datos conservados en Telegram; diferencia entre paquete y cliente Steam;
   app web sin versión de `.desktop` ni actualización ficticia.
9. Distinción entre instalada y aplicada a un proceso abierto, sin cierres
   automáticos de aplicaciones ni interferencia con Voxtype activo.
10. Manifiesto válido, QML validado con los imports instalados y ausencia de
    errores nuevos del shell. Apertura, cierre, teclado, scroll, tema,
    desactivación, reactivación, reinicio del shell y retirada funcionales.

Se utilizan pruebas con proveedores y ejecutores simulados para fallos,
concurrencia y operaciones administrativas. En Korax se ejecutan descubrimiento
y comprobación reales, validación del plugin y aceptación visual. Una prueba de
actualización real solo instala una versión nueva comprobada y se realiza con
el plan visible correspondiente; no se degrada una app para fabricar una prueba.

## 14. Referencias y estado de revisión

- [Catálogo](https://plugins.omarchy.org/index.html) y
  [guía de desarrollo](https://plugins.omarchy.org/develop.html).
- [Contrato del shell](https://github.com/omacom/omarchy/blob/quattro/docs/omarchy-shell.md).
  La API disponible en Korax se verifica contra la copia instalada del shell.
- [Actualizaciones de Arch](https://wiki.archlinux.org/title/Pacman#Upgrading_packages).
- [checkupdates](https://man.archlinux.org/man/checkupdates.8.en): usa una base
  separada; salida 2 significa que no hay actualizaciones, no un fallo de red.
- [Referencia de Flatpak](https://docs.flatpak.org/en/latest/flatpak-command-reference.html).
- Evidencia local de alcance: `~/.local/share/ai-updaters/updater.py` y su README;
  inventario fechado en `/home/korvo/Documents/Inventarios/2026-10-11/`.

Revisión propia: propósito y límites explícitos, integración consistente con el
shell instalado, responsabilidades separadas, estados definidos y alcance de
transacciones visible. El usuario aprobó continuar con el plan de implementación.
La revisión de ese plan y la elección del método preceden a su ejecución.
