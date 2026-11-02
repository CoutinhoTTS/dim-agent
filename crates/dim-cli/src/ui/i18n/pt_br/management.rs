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
        "models" => "modelos",
        "command rules" => "Regras de comando",
        "enabled" => "ativado",
        "shadowed by {name}" => "ocultado por {name}",
        "Reloaded." => "Recarregado.",
        "Enabled {name}." => "{name} ativado.",
        "Disabled {name}." => "{name} desativado.",
        "Reloaded {area}: {entries} entries, {disabled} disabled." => {
            "{area} recarregado: {entries} entradas, {disabled} desativadas."
        }
        "Reloaded {area}: {entries} entries." => "{area} recarregado: {entries} entradas.",
        "Hooks" => "Hooks",
        "invalid" => "inválido",
        "Saved." => "Salvo.",
        "Saved {name}." => "{name} salvo.",
        "No change." => "Nenhuma alteração.",
        "Your text is kept in {path}." => "Seu texto está guardado em {path}.",
        "The user's own document; always in effect." => {
            "O documento do próprio usuário; sempre em vigor."
        }
        "There is no hooks file there." => "Não há arquivo de hooks ali.",
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
        "This daemon lists {area} but cannot toggle them." => {
            "O daemon lista {area}, mas não permite alternar."
        }
        "This daemon lists {area} but cannot edit them." => {
            "O daemon lista {area}, mas não permite editar."
        }
        "This daemon lists {area} but cannot reload them." => {
            "O daemon lista {area}, mas não permite recarregar."
        }
        "This daemon lists {area} but cannot add them." => {
            "O daemon lista {area}, mas não permite adicionar."
        }
        "This daemon lists {area} but cannot remove them." => {
            "O daemon lista {area}, mas não permite remover."
        }
        "This daemon lists {area} but cannot run them." => {
            "O daemon lista {area}, mas não permite executar."
        }
        "This daemon lists {area} but cannot inspect them." => {
            "O daemon lista {area}, mas não permite ver os detalhes ou o histórico."
        }
        "The document deleted {field}, which the owner cannot clear; nothing was written." => {
            "O documento removeu {field}, que o proprietário não pode limpar; nada foi escrito."
        }
        "The document changes no field; nothing was written." => {
            "O documento não altera nenhum campo; nada foi escrito."
        }
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
        "The models changed since they were read; nothing was removed." => {
            "Os modelos mudaram desde a leitura; nada foi removido."
        }
        "The models changed since they were read; nothing was written." => {
            "Os modelos mudaram desde a leitura; nada foi escrito."
        }
        "No provider has that id." => "Nenhum provedor tem esse id.",
        "{name} is unavailable ({reason}); its enablement is kept as it is." => {
            "{name} não está disponível ({reason}); sua ativação permanece como está."
        }
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
        "not supported" => "sem suporte",
        "supported" => "com suporte",
        "Command rules" => "Regras de comando",
        "user" => "usuário",
        "Matches only this command." => "Corresponde apenas a este comando.",
        "Matches commands whose first words are these." => {
            "Corresponde a comandos cujas primeiras palavras são estas."
        }
        "Allows a simple command that would otherwise ask." => {
            "Permite um comando simples que de outra forma perguntaria."
        }
        "Denies the command under every preset." => "Nega o comando em qualquer predefinição.",
        "Applies in every Session, in every workspace." => {
            "Aplica-se em toda Sessão e em todo workspace."
        }
        "Rule added: {rule}." => "Regra adicionada: {rule}.",
        "Rule already present: {rule}." => "Regra já presente: {rule}.",
        "Rule removed: {rule}." => "Regra removida: {rule}.",
        "That command rule is no longer listed; nothing was removed." => {
            "Essa regra de comando não está mais listada; nada foi removido."
        }
        "Command rules changed since they were read; nothing was removed." => {
            "As regras de comando mudaram desde que foram lidas; nada foi removido."
        }
        "A command rule is DECISION MATCH COMMAND, such as deny prefix git push." => {
            "Uma regra de comando é escrita como DECISION MATCH COMMAND, por exemplo deny prefix git push."
        }
        "Command: {command}" => "Comando: {command}",
        "Use the line prompt or dimcode permissions." => {
            "Use o prompt de linha ou dimcode permissions."
        }
        _ => return None,
    })
}
