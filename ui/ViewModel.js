.pragma library

function array(value) {return Array.isArray(value) ? value : []}
function excluded(app) {
  var protectedNames = ["codex", "codex-bin", "claude", "claude-code", "claude-desktop", "antigravity", "opencode", "hermes", "hermes-agent", "chatgpt", "chatgpt-omarchy"]
  return array(app.components).some(function(t) {return t.locator && protectedNames.indexOf(t.locator.package) >= 0})
}
function checksFor(report, app) {return array(report.checks).filter(function(c) {return array(app.components).some(function(t) {return t.id === c.target_id})})}
function canEnrollTelegram(app, check) {return app && check && !check.installed && !check.stale && check.available && check.available.kind === "product" && array(app.components).some(function(t) {return t.id === check.target_id && t.locator.kind === "telegram_local"})}
function updatable(report, app) {return checksFor(report,app).some(function(c) {return c.state === "update_available" && !c.stale})}
function visibleRows(report, filter, query) {
  report=report||{};query=String(query||"").toLocaleLowerCase()
  return array(report.apps).filter(function(app) {
    return !excluded(app) && (filter !== "selected" || array(report.selected).indexOf(app.id) >= 0)
      && (filter !== "updates" || updatable(report,app))
      && (!query || (app.name+" "+app.id).toLocaleLowerCase().indexOf(query)>=0)
  }).sort(function(a,b) {return String(a.name).localeCompare(String(b.name))})
}
function updateCount(report) {
  if (!report || !report.coverage || report.coverage.status!=="verified") return 0
  return visibleRows(report,"selected","").filter(function(app) {return updatable(report,app)}).length
}
function stateLabel(check) {
  if (!check) return "Sin comprobar"
  if (check.stale) return "Dato anterior · fuente sin comprobar"
  return ({current:"Actualizada en su canal",update_available:"Actualización disponible",publisher_ahead:"Fabricante más reciente",installed_ahead:"Instalada posterior al canal",unknown:"Versión por comprobar",unsupported:"Requiere gestión manual",web_automatic:"Actualización del servicio web"})[check.state]||"Sin comprobar"
}
function version(value) {
  if (!value) return "Sin versión verificada"
  if(value.kind==="flatpak_commit") return "commit "+String(value.raw).slice(0,12)
  return String(value.raw)
}
function checkedLabel(report) {
  if(!report || !report.checked_at) return "Aún no se han comprobado las fuentes"
  return "Comprobado: "+new Date(report.checked_at*1000).toLocaleString()
}
function terminal(phase) {return ["completed","failed","cancelled"].indexOf(phase)>=0}
function phaseLabel(phase) {return ({prepared:"Plan preparado",queued:"Trabajo recibido",downloading:"Descargando",building:"Compilando",awaiting_auth:"Esperando autorización",awaiting_decision:"Esperando tu decisión",installing:"Instalando",verifying:"Verificando",completed:"Trabajo terminado",failed:"Trabajo con errores",cancelled:"Trabajo cancelado"})[phase]||"Sin trabajo activo"}
function outcomeLabel(outcome) {return ({updated:"Actualizada",current:"Ya estaba actualizada",needs_reopen:"Instalada · pendiente de reabrir",failed:"Falló la verificación",skipped:"Pendiente de un plan nuevo"})[outcome]||outcome}
function planText(plan) {
  if(!plan) return ""
  var lines=array(plan.notices).slice()
  array(plan.steps).forEach(function(s) {
    lines.push("\n"+({arch_full:"Arch · alcance completo",aur_build:"AUR · receta y artefactos",flatpak_update:"Flatpak · app y runtimes",telegram_replace:"Telegram · ejecutables locales",steam_update:"Steam · cliente Valve"})[s.kind])
    array(s.package_changes).forEach(function(p) {lines.push(p.name+": "+(p.old_version||"sin instalar")+" → "+p.new_version+" · "+(({system:"sistema",app:"aplicación",dependency:"dependencia",build_dependency:"compilación"})[p.reason]||p.reason)+(p.action==="remove"?" · RETIRADA":""))})
    array(s.flatpak_changes).forEach(function(c) {lines.push(c.reference+" · "+(c.installation.kind==="user"?"usuario":"sistema")+" · "+c.remote+" · "+c.commit.slice(0,12))})
    array(s.downloads).forEach(function(d) {lines.push(d.url)})
    if(s.aur_recipe) lines.push("Commit: "+s.aur_recipe.commit+"\nReceta SHA-256: "+s.aur_recipe.digest)
    array(s.built_artifacts).forEach(function(a) {lines.push(a.name+" "+a.version+"\nSHA-256: "+a.sha256)})
  })
  return lines.join("\n")
}
function safeLink(value) {
  var url=String(value||"")
  if(!/^https:\/\/[^\s@]+$/.test(url)) return false
  var host=url.slice(8).split("/")[0]
  if(["telegram.org","td.telegram.org","desktop.telegram.org","voxtype.io","developer.chrome.com","ghostty.org","mozilla.org","www.thunderbird.net","code.visualstudio.com","remmina.org","aur.archlinux.org","www.audacityteam.org","protege.stanford.edu","client-update.akamai.steamstatic.com"].indexOf(host)>=0) return true
  return /^https:\/\/github\.com\/(telegramdesktop\/tdesktop|ghostty-org\/ghostty|audacity\/audacity|protegeproject\/protege|peteonrails\/voxtype)(\/|$)/.test(url)
    || /^https:\/\/gitlab\.com\/Remmina\/Remmina(\/|$)/.test(url)
}

function appForTarget(report,id) {return report&&array(report.apps).filter(function(a){return array(a.components).some(function(t){return t.id===id})})[0]||null}
function isVoxtype(app) {return app&&array(app.components).some(function(t){return t.locator&&/^voxtype/.test(t.locator.package||"")})}
function sourceLinks(check) {
    var links=[];
    if(check&&safeLink(check.source))links.push(check.source);
    array(check&&check.details).forEach(function(d){var prefix="Fuente del fabricante: ";if(d.indexOf(prefix)===0&&safeLink(d.slice(prefix.length)))links.push(d.slice(prefix.length))});
    return links;
}
