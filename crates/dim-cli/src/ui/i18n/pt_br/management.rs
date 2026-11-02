pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Loading..." => "Carregando…",
        "No entries." => "Nenhuma entrada.",
        "No matching entry." => "Nenhuma entrada correspondente.",
        "Issues" => "Problemas",
        "sources" => "Fontes",
        "skills" => "Skills",
        "hooks" => "hooks",
        "scheduled jobs" => "tarefas agendadas",
        "enabled" => "ativado",
        "shadowed by {name}" => "ocultado por {name}",
        "Reloaded." => "Recarregado.",
        "Enabled {name}." => "{name} ativado.",
        "Disabled {name}." => "{name} desativado.",
        "Reloaded {area}: {entries} entries, {disabled} disabled." => {
            "{area} recarregado: {entries} entradas, {disabled} desativadas."
        }
        "Hooks" => "Hooks",
        "invalid" => "inválido",
        "Saved." => "Salvo.",
        "Saved {name}." => "{name} salvo.",
        "No change." => "Nenhuma alteração.",
        "Your text is kept in {path}." => "Seu texto está guardado em {path}.",
        "The user's own document; always in effect." => {
            "O documento do próprio usuário; sempre em vigor."
        }
        "No .dimcode/hooks.json in this workspace." => {
            "Não há .dimcode/hooks.json neste espaço de trabalho."
        }
        "Owned by plugin {plugin}." => "Pertence ao plugin {plugin}.",
        "The user document turns every hook off (disableAllHooks)." => {
            "O documento do usuário desativa todos os hooks (disableAllHooks)."
        }
        "Enter toggle" => "Enter alternar",
        "Ctrl+E edit" => "Ctrl+E editar",
        "Ctrl+N add" => "Ctrl+N adicionar",
        "Ctrl+R reload" => "Ctrl+R recarregar",
        "Ctrl+T run" => "Ctrl+T executar",
        "Ctrl+X remove" => "Ctrl+X remover",
        "Tab inspect" => "Tab inspecionar",
        "Remove {name}? Enter confirm · Esc cancel" => {
            "Remover {name}? Enter confirmar · Esc cancelar"
        }
        "↑↓ scroll · Esc back" => "↑↓ rolar · Esc voltar",
        "Added." => "Adicionado.",
        "Removed {name}." => "{name} removido.",
        "Ran {name}." => "{name} executado.",
        "The user document changed since it was read; nothing was written." => {
            "O documento do usuário mudou desde que foi lido; nada foi escrito."
        }
        "The job changed since it was read; nothing was written." => {
            "A tarefa mudou desde que foi lida; nada foi escrito."
        }
        "The jobs changed since they were read; nothing was written." => {
            "As tarefas mudaram desde que foram lidas; nada foi escrito."
        }
        "Your next edit opens the current document." => {
            "Sua próxima edição abre o documento atual."
        }
        "Your next add reopens your text." => "Sua próxima adição abre seu texto novamente.",
        "Scheduler" => "Agendador",
        "jobs" => "tarefas",
        "Added {name}." => "{name} adicionado.",
        "Run {name}: {outcome}." => "{name} executado: {outcome}.",
        "Recent runs of {name}" => "Execuções recentes de {name}",
        "No run yet." => "Nenhuma execução ainda.",
        "No next run." => "Nenhuma próxima execução.",
        "Next run {instant}." => "Próxima execução {instant}.",
        "Last run {outcome} at {instant}." => "Última execução {outcome} em {instant}.",
        "every {minutes} minutes" => "a cada {minutes} minutos",
        "at {instant}" => "em {instant}",
        "cron {expression}" => "cron {expression}",
        "cron {expression} in {zone}" => "cron {expression} em {zone}",
        "The job changed since it was read; nothing was removed." => {
            "A tarefa mudou desde que foi lida; nada foi removido."
        }
        "The Scheduler has no area document." => "O Agendador não tem documento de área.",
        "No scheduled job has that id." => "Nenhuma tarefa agendada tem esse id.",
        "This daemon cannot manage {area}." => "Não é possível gerenciar {area}.",
        "The document deleted {field}, which the owner cannot clear; nothing was written." => {
            "O documento removeu {field}, que o proprietário não pode limpar; nada foi escrito."
        }
        "The document changes no field; nothing was written." => {
            "O documento não altera nenhum campo; nada foi escrito."
        }
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "As configurações de hooks deste espaço de trabalho mudaram desde que foram lidas; nada foi escrito."
        }
        _ => return None,
    })
}
