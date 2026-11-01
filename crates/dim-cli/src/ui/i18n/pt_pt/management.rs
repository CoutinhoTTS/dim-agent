pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Loading..." => "Carregando…",
        "No entries." => "Nenhuma entrada.",
        "No matching entry." => "Nenhuma entrada correspondente.",
        "Issues" => "Problemas",
        "sources" => "Fontes",
        "skills" => "Competências",
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
        "Saved." => "Guardado.",
        "No change." => "Nenhuma alteração.",
        "Your text is kept in {path}." => "O seu texto está guardado em {path}.",
        "The user's own document; always in effect." => {
            "O documento do próprio utilizador; sempre em vigor."
        }
        "No .dimcode/hooks.json in this workspace." => {
            "Não há .dimcode/hooks.json neste espaço de trabalho."
        }
        "Owned by plugin {plugin}." => "Pertence ao plugin {plugin}.",
        "The user document turns every hook off (disableAllHooks)." => {
            "O documento do utilizador desativa todos os hooks (disableAllHooks)."
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
        "↑↓ scroll · Esc back" => "↑↓ deslocar · Esc voltar",
        "Added." => "Adicionado.",
        "Removed {name}." => "{name} removido.",
        "Ran {name}." => "{name} executado.",
        "The user document changed since it was read; nothing was written. Your next edit opens the current document." => {
            "O documento do utilizador mudou desde que foi lido; nada foi escrito. A sua próxima edição abre o documento atual."
        }
        "Scheduler" => "Agendador",
        "Added {name}." => "{name} adicionado.",
        "Run {name}: {outcome}." => "{name} executado: {outcome}.",
        "Recent runs of {name}" => "Execuções recentes de {name}",
        "No run yet." => "Ainda não há execuções.",
        "No next run." => "Não há próxima execução.",
        "Next run {instant}." => "Próxima execução {instant}.",
        "Last run {outcome} at {instant}." => "Última execução {outcome} em {instant}.",
        "every {minutes} minutes" => "a cada {minutes} minutos",
        "at {instant}" => "em {instant}",
        "cron {expression}" => "cron {expression}",
        "cron {expression} in {zone}" => "cron {expression} em {zone}",
        "The job changed since it was read; nothing was written. Your next edit opens the current job." => {
            "A tarefa mudou desde que foi lida; nada foi escrito. A sua próxima edição abre a tarefa atual."
        }
        "The jobs changed since they were read; nothing was written. Your next add opens your text again." => {
            "As tarefas mudaram desde que foram lidas; nada foi escrito. A sua próxima adição abre novamente o seu texto."
        }
        "The job changed since it was read; nothing was removed." => {
            "A tarefa mudou desde que foi lida; nada foi removido."
        }
        "The Scheduler has no area document." => "O Agendador não tem documento de área.",
        "No scheduled job has that id." => "Nenhuma tarefa agendada tem esse id.",
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "As definições de hooks deste espaço de trabalho mudaram desde que foram lidas; nada foi escrito."
        }
        "Workflow" => "Fluxo de trabalho",
        "Saved {name}." => "{name} guardado.",
        "Saved {name}. It stays disabled until you enable it." => {
            "{name} guardado. Permanece desativado até o ativar."
        }
        "The definitions changed since they were read; nothing was written." => {
            "As definições mudaram desde que foram lidas; nada foi escrito."
        }
        "The definition changed since it was read; nothing was written. Your next edit opens the current definition." => {
            "A definição mudou desde que foi lida; nada foi escrito. A sua próxima edição abre a definição atual."
        }
        "The definitions changed since they were read; nothing was written. Your next add opens your text again." => {
            "As definições mudaram desde que foram lidas; nada foi escrito. A sua próxima adição abre o seu texto novamente."
        }
        "A definition's name is its key; add it to save it under another name." => {
            "O nome de uma definição é a sua chave; adicione-a para a guardar com outro nome."
        }
        "No project definition is named {name}." => "Nenhuma definição de projeto se chama {name}.",
        "No workflow definition is named {name}." => {
            "Nenhuma definição de fluxo de trabalho se chama {name}."
        }
        "Saved in the user workflow source; always enabled." => {
            "Guardado na fonte de fluxos de trabalho do utilizador; sempre ativado."
        }
        "Starts only after you enable it." => "Só inicia depois de o ativar.",
        "Defined by package {package}." => "Definido pelo pacote {package}.",
        "{name} — {description} (required)" => "{name} — {description} (obrigatório)",
        "{name} — {description}" => "{name} — {description}",
        "{name} — required" => "{name} — obrigatório",
        "{name} — default: {default}" => "{name} — predefinição: {default}",
        "and {count} more" => "e mais {count}",
        "Recent runs" => "Execuções recentes",
        "A workflow definition is saved as an object with a source and a document." => {
            "Uma definição de fluxo de trabalho é guardada como um objeto com uma fonte e um documento."
        }
        "A definition is saved to the user source or the project source." => {
            "Uma definição é guardada na fonte do utilizador ou na fonte do projeto."
        }
        _ => return None,
    })
}
