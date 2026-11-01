pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Loading..." => "Cargando…",
        "No entries." => "No hay entradas.",
        "No matching entry." => "Ninguna entrada coincide.",
        "Issues" => "Problemas",
        "sources" => "Fuentes",
        "skills" => "Skills",
        "enabled" => "activado",
        "shadowed by {name}" => "ocultado por {name}",
        "Reloaded." => "Recargado.",
        "Enabled {name}." => "{name} activado.",
        "Disabled {name}." => "{name} desactivado.",
        "Reloaded {area}: {entries} entries, {disabled} disabled." => {
            "{area} recargado: {entries} entradas, {disabled} desactivadas."
        }
        "Hooks" => "Hooks",
        "invalid" => "no válido",
        "Saved." => "Guardado.",
        "Saved {name}." => "{name} guardado.",
        "No change." => "Sin cambios.",
        "Your text is kept in {path}." => "Tu texto se conserva en {path}.",
        "The user's own document; always in effect." => {
            "El documento propio del usuario; siempre en efecto."
        }
        "No .dimcode/hooks.json in this workspace." => {
            "No hay .dimcode/hooks.json en este espacio de trabajo."
        }
        "Owned by plugin {plugin}." => "Propiedad del plugin {plugin}.",
        "The user document turns every hook off (disableAllHooks)." => {
            "El documento del usuario desactiva todos los hooks (disableAllHooks)."
        }
        "Enter toggle" => "Enter alternar",
        "Ctrl+E edit" => "Ctrl+E editar",
        "Ctrl+N add" => "Ctrl+N añadir",
        "Ctrl+R reload" => "Ctrl+R recargar",
        "Ctrl+T run" => "Ctrl+T ejecutar",
        "Ctrl+X remove" => "Ctrl+X eliminar",
        "Tab inspect" => "Tab inspeccionar",
        "Remove {name}? Enter confirm · Esc cancel" => {
            "¿Eliminar {name}? Enter confirmar · Esc cancelar"
        }
        "↑↓ scroll · Esc back" => "↑↓ desplazar · Esc volver",
        "Added." => "Añadido.",
        "Removed {name}." => "{name} eliminado.",
        "Ran {name}." => "{name} ejecutado.",
        "The user document changed since it was read; nothing was written." => {
            "El documento del usuario cambió desde que se leyó; no se escribió nada."
        }
        "The job changed since it was read; nothing was written." => {
            "La tarea cambió desde que se leyó; no se escribió nada."
        }
        "The jobs changed since they were read; nothing was written." => {
            "Las tareas cambiaron desde que se leyeron; no se escribió nada."
        }
        "Your next edit opens the current document." => {
            "Tu próxima edición abre el documento actual."
        }
        "Your next add reopens your text." => {
            "Tu próxima acción de añadir vuelve a abrir tu texto."
        }
        "Scheduler" => "Programador",
        "jobs" => "tareas",
        "Added {name}." => "{name} añadido.",
        "Run {name}: {outcome}." => "{name} ejecutado: {outcome}.",
        "Recent runs of {name}" => "Ejecuciones recientes de {name}",
        "No run yet." => "Todavía no hay ejecuciones.",
        "No next run." => "No hay próxima ejecución.",
        "Next run {instant}." => "Próxima ejecución {instant}.",
        "Last run {outcome} at {instant}." => "Última ejecución {outcome} a las {instant}.",
        "every {minutes} minutes" => "cada {minutes} minutos",
        "at {instant}" => "a las {instant}",
        "cron {expression}" => "cron {expression}",
        "cron {expression} in {zone}" => "cron {expression} en {zone}",
        "The job changed since it was read; nothing was removed." => {
            "La tarea cambió desde que se leyó; no se eliminó nada."
        }
        "The Scheduler has no area document." => "El Programador no tiene documento de área.",
        "No scheduled job has that id." => "Ninguna tarea programada tiene ese id.",
        "This daemon cannot manage {area}." => "No se puede gestionar {area}.",
        "The document deleted {field}, which the owner cannot clear; nothing was written." => {
            "El documento elimina {field}, que el propietario no puede borrar; no se escribió nada."
        }
        "The document changes no field; nothing was written." => {
            "El documento no cambia ningún campo; no se escribió nada."
        }
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "La configuración de hooks de este espacio de trabajo cambió desde que se leyó; no se escribió nada."
        }
        _ => return None,
    })
}
