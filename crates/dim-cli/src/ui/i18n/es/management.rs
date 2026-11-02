pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Loading..." => "Cargando…",
        "No entries." => "No hay entradas.",
        "No matching entry." => "Ninguna entrada coincide.",
        "Issues" => "Problemas",
        "sources" => "Fuentes",
        "skills" => "Skills",
        "hooks" => "hooks",
        "scheduled jobs" => "tareas programadas",
        "models" => "modelos",
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
        "There is no hooks file there." => "No hay ningún archivo de hooks allí.",
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
        "unavailable" => "no disponible",
        "Reset {name}? Enter confirm · Esc cancel" => {
            "¿Restablecer {name}? Enter confirmar · Esc cancelar"
        }
        "Reset {name} to its catalog entry." => "{name} restablecido a su entrada del catálogo.",
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
        "This daemon lists {area} but cannot toggle them." => {
            "El daemon lista {area}, pero no permite alternar."
        }
        "This daemon lists {area} but cannot edit them." => {
            "El daemon lista {area}, pero no permite editar."
        }
        "This daemon lists {area} but cannot reload them." => {
            "El daemon lista {area}, pero no permite recargar."
        }
        "This daemon lists {area} but cannot add them." => {
            "El daemon lista {area}, pero no permite añadir."
        }
        "This daemon lists {area} but cannot remove them." => {
            "El daemon lista {area}, pero no permite eliminar."
        }
        "This daemon lists {area} but cannot run them." => {
            "El daemon lista {area}, pero no permite ejecutar."
        }
        "This daemon lists {area} but cannot inspect them." => {
            "El daemon lista {area}, pero no permite ver sus detalles o su historial."
        }
        "The document deleted {field}, which the owner cannot clear; nothing was written." => {
            "El documento elimina {field}, que el propietario no puede borrar; no se escribió nada."
        }
        "The document changes no field; nothing was written." => {
            "El documento no cambia ningún campo; no se escribió nada."
        }
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "La configuración de hooks de este espacio de trabajo cambió desde que se leyó; no se escribió nada."
        }
        "Models" => "Modelos",
        "catalog" => "catálogo",
        "overrides" => "anulaciones",
        "added" => "añadidos",
        "Models · {name}" => "Modelos · {name}",
        "undeclared" => "sin declarar",
        "Context window {window} · max output {output}" => {
            "Ventana de contexto {window} · salida máx. {output}"
        }
        "Endpoint {endpoint}" => "Punto de conexión {endpoint}",
        "Endpoint {endpoint} · override {value}" => {
            "Punto de conexión {endpoint} · anulación {value}"
        }
        "The provider reports it unavailable." => "El proveedor lo informa como no disponible.",
        "Limits undeclared; edit the model to declare them." => {
            "Límites sin declarar; edita el modelo para declararlos."
        }
        "Sign in to the account to use its models." => {
            "Inicia sesión en la cuenta para usar sus modelos."
        }
        "The signed-in account changed; reload to list its models." => {
            "La cuenta iniciada cambió; recarga para listar sus modelos."
        }
        "The provider has no catalog yet." => "Este proveedor aún no tiene catálogo.",
        "The provider listed no models." => "El proveedor no listó ningún modelo.",
        "Listed {count} models." => "Se listaron {count} modelos.",
        "Refreshed the models." => "Modelos actualizados.",
        "Could not list models: {error}. Showing the stored list." => {
            "No se pudieron listar los modelos: {error}. Se muestra la lista guardada."
        }
        "The signed-in account changed; the refresh cleared preferences, overrides and disables." => {
            "La cuenta iniciada cambió; la recarga borró preferencias, anulaciones y desactivaciones."
        }
        "The models changed since they were read; nothing was removed." => {
            "Los modelos cambiaron desde su lectura; no se eliminó nada."
        }
        "The models changed since they were read; nothing was written." => {
            "Los modelos cambiaron desde su lectura; no se escribió nada."
        }
        "No provider has that id." => "Ningún proveedor tiene ese id.",
        "{name} is unavailable ({reason}); its enablement is kept as it is." => {
            "{name} no está disponible ({reason}); su activación se mantiene como está."
        }
        "No model has that id." => "Ningún modelo tiene ese id.",
        "This model cannot be removed." => "Este modelo no se puede eliminar.",
        "A Provider's models have no area document." => {
            "Los modelos de un proveedor no tienen documento de área."
        }
        "Models have no run operation." => "Los modelos no tienen operación de ejecución.",
        "The document is not an object; nothing was written." => {
            "El documento no es un objeto; no se escribió nada."
        }
        "The document changes no editable field; nothing was written." => {
            "El documento no cambia ningún campo editable; no se escribió nada."
        }
        "The model document names no model." => "El documento del modelo no nombra ningún modelo.",
        "The provider has no model catalog to write." => {
            "Este proveedor no tiene catálogo de modelos que escribir."
        }
        "ID {id}" => "ID {id}",
        "Source {source}" => "Origen {source}",
        "Status {status}" => "Estado {status}",
        "Tools {value}" => "Herramientas {value}",
        "Vision {value}" => "Visión {value}",
        "Reasoning {value}" => "Razonamiento {value}",
        "Modalities {list}" => "Modalidades {list}",
        "Description {text}" => "Descripción {text}",
        "Release date {date}" => "Fecha de lanzamiento {date}",
        "Pricing {value}" => "Precio {value}",
        "Rate limit {value}" => "Límite de tasa {value}",
        "Matched provider {id}" => "Proveedor coincidente {id}",
        "Matched model {id}" => "Modelo coincidente {id}",
        "Match strategy {strategy}" => "Estrategia de coincidencia {strategy}",
        "Capability overrides {fields}" => "Anulaciones de capacidades {fields}",
        "yes" => "sí",
        "no" => "no",
        "not supported" => "no compatible",
        "supported" => "compatible",
        _ => return None,
    })
}
