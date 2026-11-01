pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Loading..." => "Chargement…",
        "No entries." => "Aucune entrée.",
        "No matching entry." => "Aucune entrée correspondante.",
        "Issues" => "Problèmes",
        "sources" => "Sources",
        "skills" => "Skills",
        "enabled" => "activé",
        "shadowed by {name}" => "masqué par {name}",
        "Reloaded." => "Rechargé.",
        "Enabled {name}." => "{name} activé.",
        "Disabled {name}." => "{name} désactivé.",
        "Reloaded {area}: {entries} entries, {disabled} disabled." => {
            "{area} rechargé : {entries} entrées, {disabled} désactivées."
        }
        "Hooks" => "Hooks",
        "invalid" => "non valide",
        "Saved." => "Enregistré.",
        "No change." => "Aucun changement.",
        "Your text is kept in {path}." => "Votre texte est conservé dans {path}.",
        "The user's own document; always in effect." => {
            "Le document de l'utilisateur ; toujours actif."
        }
        "No .dimcode/hooks.json in this workspace." => {
            "Pas de .dimcode/hooks.json dans cet espace de travail."
        }
        "Owned by plugin {plugin}." => "Appartient au plugin {plugin}.",
        "The user document turns every hook off (disableAllHooks)." => {
            "Le document utilisateur désactive tous les hooks (disableAllHooks)."
        }
        "Enter toggle" => "Enter basculer",
        "Ctrl+E edit" => "Ctrl+E modifier",
        "Ctrl+N add" => "Ctrl+N ajouter",
        "Ctrl+R reload" => "Ctrl+R recharger",
        "Ctrl+T run" => "Ctrl+T exécuter",
        "Ctrl+X remove" => "Ctrl+X supprimer",
        "Tab inspect" => "Tab inspecter",
        "Remove {name}? Enter confirm · Esc cancel" => {
            "Supprimer {name} ? Enter confirmer · Esc annuler"
        }
        "↑↓ scroll · Esc back" => "↑↓ défiler · Esc revenir",
        "Added." => "Ajouté.",
        "Removed {name}." => "{name} supprimé.",
        "Ran {name}." => "{name} exécuté.",
        "The user document changed since it was read; nothing was written. Your next edit opens the current document." => {
            "Le document utilisateur a changé depuis sa lecture ; rien n'a été écrit. Votre prochaine modification ouvrira le document actuel."
        }
        "Scheduler" => "Planificateur",
        "Added {name}." => "{name} ajouté.",
        "Run {name}: {outcome}." => "{name} exécuté : {outcome}.",
        "Recent runs of {name}" => "Exécutions récentes de {name}",
        "No run yet." => "Aucune exécution pour l'instant.",
        "No next run." => "Aucune exécution à venir.",
        "Next run {instant}." => "Prochaine exécution {instant}.",
        "Last run {outcome} at {instant}." => "Dernière exécution {outcome} à {instant}.",
        "every {minutes} minutes" => "toutes les {minutes} minutes",
        "at {instant}" => "à {instant}",
        "cron {expression}" => "cron {expression}",
        "cron {expression} in {zone}" => "cron {expression} en {zone}",
        "The job changed since it was read; nothing was written. Your next edit opens the current job." => {
            "La tâche a changé depuis sa lecture ; rien n'a été écrit. Votre prochaine modification ouvrira la tâche actuelle."
        }
        "The jobs changed since they were read; nothing was written. Your next add opens your text again." => {
            "Les tâches ont changé depuis leur lecture ; rien n'a été écrit. Votre prochain ajout rouvrira votre texte."
        }
        "The job changed since it was read; nothing was removed." => {
            "La tâche a changé depuis sa lecture ; rien n'a été supprimé."
        }
        "The Scheduler has no area document." => "Le planificateur n'a pas de document de domaine.",
        "No scheduled job has that id." => "Aucune tâche planifiée n'a cet id.",
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "Les réglages des hooks de cet espace de travail ont changé depuis leur lecture ; rien n'a été écrit."
        }
        "Command rules" => "Règles de commande",
        "user" => "utilisateur",
        "Matches only this command." => "Correspond uniquement à cette commande.",
        "Matches commands whose first words are these." => {
            "Correspond aux commandes qui commencent par ces mots."
        }
        "Allows a simple command that would otherwise ask." => {
            "Autorise une commande simple qui serait autrement demandée."
        }
        "Denies the command under every preset." => {
            "Refuse la commande quel que soit le préréglage."
        }
        "Applies in every Session, in every workspace." => {
            "S'applique à chaque session et à chaque espace de travail."
        }
        "Rule added: {rule}." => "Règle ajoutée : {rule}.",
        "Rule already present: {rule}." => "Règle déjà présente : {rule}.",
        "Rule removed: {rule}." => "Règle supprimée : {rule}.",
        "That command rule is no longer listed; nothing was removed." => {
            "Cette règle de commande n'est plus répertoriée ; rien n'a été supprimé."
        }
        "Command rules changed since they were read; nothing was removed." => {
            "Les règles de commande ont changé depuis leur lecture ; rien n'a été supprimé."
        }
        "A command rule is DECISION MATCH COMMAND, such as deny prefix git push." => {
            "Une règle de commande s'écrit DECISION MATCH COMMAND, par exemple deny prefix git push."
        }
        "Use the line prompt or dimcode permissions." => {
            "Utilisez l'invite de ligne ou dimcode permissions."
        }
        _ => return None,
    })
}
