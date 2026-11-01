pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Loading..." => "Carregando…",
        "No entries." => "Nenhuma entrada.",
        "No matching entry." => "Nenhuma entrada correspondente.",
        "Issues" => "Problemas",
        "sources" => "Fontes",
        "skills" => "Skills",
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
        "unavailable" => "indisponível",
        "Reset {name}? Enter confirm · Esc cancel" => {
            "Restaurar {name}? Enter confirmar · Esc cancelar"
        }
        "Reset {name} to its catalog entry." => "{name} restaurado para a entrada do catálogo.",
        "Ran {name}." => "{name} executado.",
        "The user document changed since it was read; nothing was written. Your next edit opens the current document." => {
            "O documento do usuário mudou desde que foi lido; nada foi escrito. Sua próxima edição abre o documento atual."
        }
        "Scheduler" => "Agendador",
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
        "The job changed since it was read; nothing was written. Your next edit opens the current job." => {
            "A tarefa mudou desde que foi lida; nada foi escrito. Sua próxima edição abre a tarefa atual."
        }
        "The jobs changed since they were read; nothing was written. Your next add opens your text again." => {
            "As tarefas mudaram desde que foram lidas; nada foi escrito. Sua próxima adição abre seu texto novamente."
        }
        "The job changed since it was read; nothing was removed." => {
            "A tarefa mudou desde que foi lida; nada foi removido."
        }
        "The Scheduler has no area document." => "O Agendador não tem documento de área.",
        "No scheduled job has that id." => "Nenhuma tarefa agendada tem esse id.",
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "As configurações de hooks deste espaço de trabalho mudaram desde que foram lidas; nada foi escrito."
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
        "The provider reports it unavailable." => "O provedor o informa como indisponível.",
        "Limits undeclared; edit the model to declare them." => {
            "Limites não declarados; edite o modelo para declará-los."
        }
        "Sign in to the account to use its models." => "Entre na conta para usar os modelos dela.",
        "The signed-in account changed; reload to list its models." => {
            "A conta conectada mudou; recarregue para listar os modelos dela."
        }
        "The provider has no catalog yet." => "Este provedor ainda não tem catálogo.",
        "The provider listed no models." => "O provedor não listou nenhum modelo.",
        "Listed {count} models." => "{count} modelos listados.",
        "Refreshed the models." => "Modelos atualizados.",
        "Could not list models: {error}. Showing the stored list." => {
            "Não foi possível listar os modelos: {error}. Mostrando a lista salva."
        }
        "The signed-in account changed; the refresh cleared preferences, overrides and disables." => {
            "A conta conectada mudou; a recarga limpou preferências, substituições e desativações."
        }
        "The models changed since they were read; nothing was written. Your next edit opens the current list." => {
            "Os modelos mudaram desde a leitura; nada foi escrito. A próxima edição abrirá a lista atual."
        }
        "The models changed since they were read; nothing was written. Your next add opens your text again." => {
            "Os modelos mudaram desde a leitura; nada foi escrito. A próxima adição abrirá seu texto novamente."
        }
        "The models changed since they were read; nothing was removed." => {
            "Os modelos mudaram desde a leitura; nada foi removido."
        }
        "The models changed since they were read; nothing was written." => {
            "Os modelos mudaram desde a leitura; nada foi escrito."
        }
        "No provider has that id." => "Nenhum provedor tem esse id.",
        "No model has that id." => "Nenhum modelo tem esse id.",
        "This model cannot be removed." => "Este modelo não pode ser removido.",
        "A Provider's models have no area document." => {
            "Os modelos de um provedor não têm documento de área."
        }
        "Models have no run operation." => "Modelos não têm operação de execução.",
        "The document is not an object; nothing was written." => {
            "O documento não é um objeto; nada foi escrito."
        }
        "The document changes no editable field; nothing was written." => {
            "O documento não altera nenhum campo editável; nada foi escrito."
        }
        "The model document names no model." => "O documento do modelo não nomeia nenhum modelo.",
        "The provider has no model catalog to write." => {
            "Este provedor não tem catálogo de modelos para escrever."
        }
        "ID {id}" => "ID {id}",
        "Source {source}" => "Origem {source}",
        "Status {status}" => "Status {status}",
        "Tools {value}" => "Ferramentas {value}",
        "Vision {value}" => "Visão {value}",
        "Reasoning {value}" => "Raciocínio {value}",
        "Modalities {list}" => "Modalidades {list}",
        "Description {text}" => "Descrição {text}",
        "Release date {date}" => "Data de lançamento {date}",
        "Pricing {value}" => "Preço {value}",
        "Rate limit {value}" => "Limite de taxa {value}",
        "Matched provider {id}" => "Provedor correspondente {id}",
        "Matched model {id}" => "Modelo correspondente {id}",
        "Match strategy {strategy}" => "Estratégia de correspondência {strategy}",
        "Capability overrides {fields}" => "Substituições de capacidades {fields}",
        "yes" => "sim",
        "no" => "não",
        "off" => "desligado",
        "supported" => "com suporte",
        _ => return None,
    })
}
