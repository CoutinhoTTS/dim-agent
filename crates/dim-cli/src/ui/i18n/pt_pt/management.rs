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
        "unavailable" => "indisponível",
        "Reset {name}? Enter confirm · Esc cancel" => {
            "Repor {name}? Enter confirmar · Esc cancelar"
        }
        "Reset {name} to its catalog entry." => "{name} reposto na entrada do catálogo.",
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
        "Models" => "Modelos",
        "catalog" => "catálogo",
        "overrides" => "substituições",
        "added" => "adicionados",
        "Models · {name}" => "Modelos · {name}",
        "undeclared" => "não declarado",
        "Context window {window} · max output {output}" => {
            "Janela de contexto {window} · saída máx. {output}"
        }
        "Endpoint {endpoint}" => "Endpoint {endpoint}",
        "Endpoint {endpoint} · override {value}" => "Endpoint {endpoint} · substituição {value}",
        "The provider reports it unavailable." => "O fornecedor indica-o como indisponível.",
        "Limits undeclared; edit the model to declare them." => {
            "Limites não declarados; edite o modelo para os declarar."
        }
        "Sign in to the account to use its models." => {
            "Inicie sessão na conta para usar os modelos dela."
        }
        "The signed-in account changed; reload to list its models." => {
            "A conta iniciada mudou; recarregue para listar os modelos dela."
        }
        "The provider has no catalog yet." => "Este fornecedor ainda não tem catálogo.",
        "The provider listed no models." => "O fornecedor não listou nenhum modelo.",
        "Listed {count} models." => "{count} modelos listados.",
        "Refreshed the models." => "Modelos atualizados.",
        "Could not list models: {error}. Showing the stored list." => {
            "Não foi possível listar os modelos: {error}. A mostrar a lista guardada."
        }
        "The signed-in account changed; the refresh cleared preferences, overrides and disables." => {
            "A conta iniciada mudou; a recarga limpou preferências, substituições e desativações."
        }
        "The models changed since they were read; nothing was written. Your next edit opens the current list." => {
            "Os modelos mudaram desde a leitura; nada foi escrito. A próxima edição abrirá a lista atual."
        }
        "The models changed since they were read; nothing was written. Your next add opens your text again." => {
            "Os modelos mudaram desde a leitura; nada foi escrito. A próxima adição abrirá novamente o seu texto."
        }
        "The models changed since they were read; nothing was removed." => {
            "Os modelos mudaram desde a leitura; nada foi removido."
        }
        "The models changed since they were read; nothing was written." => {
            "Os modelos mudaram desde a leitura; nada foi escrito."
        }
        "No provider has that id." => "Nenhum fornecedor tem esse id.",
        "No model has that id." => "Nenhum modelo tem esse id.",
        "This model cannot be removed." => "Este modelo não pode ser removido.",
        "A Provider's models have no area document." => {
            "Os modelos de um fornecedor não têm documento de área."
        }
        "Models have no run operation." => "Os modelos não têm operação de execução.",
        "The document is not an object; nothing was written." => {
            "O documento não é um objeto; nada foi escrito."
        }
        "The document changes no editable field; nothing was written." => {
            "O documento não altera nenhum campo editável; nada foi escrito."
        }
        "The model document names no model." => "O documento do modelo não indica nenhum modelo.",
        "The provider has no model catalog to write." => {
            "Este fornecedor não tem catálogo de modelos para escrever."
        }
        "ID {id}" => "ID {id}",
        "Source {source}" => "Origem {source}",
        "Status {status}" => "Estado {status}",
        "Tools {value}" => "Ferramentas {value}",
        "Vision {value}" => "Visão {value}",
        "Reasoning {value}" => "Raciocínio {value}",
        "Modalities {list}" => "Modalidades {list}",
        "Description {text}" => "Descrição {text}",
        "Release date {date}" => "Data de lançamento {date}",
        "Pricing {value}" => "Preço {value}",
        "Rate limit {value}" => "Limite de taxa {value}",
        "Matched provider {id}" => "Fornecedor correspondente {id}",
        "Matched model {id}" => "Modelo correspondente {id}",
        "Match strategy {strategy}" => "Estratégia de correspondência {strategy}",
        "Capability overrides {fields}" => "Substituições de capacidades {fields}",
        "yes" => "sim",
        "no" => "não",
        "off" => "desligado",
        "supported" => "suportado",
        _ => return None,
    })
}
