pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Loading..." => "불러오는 중…",
        "No entries." => "항목이 없습니다.",
        "No matching entry." => "일치하는 항목이 없습니다.",
        "Issues" => "문제",
        "sources" => "소스",
        "skills" => "스킬",
        "enabled" => "활성",
        "shadowed by {name}" => "{name}에 의해 가려짐",
        "Reloaded." => "다시 로드했습니다.",
        "Enabled {name}." => "{name}을(를) 활성화했습니다.",
        "Disabled {name}." => "{name}을(를) 비활성화했습니다.",
        "Reloaded {area}: {entries} entries, {disabled} disabled." => {
            "{area} 다시 로드됨: 항목 {entries}개, 비활성 {disabled}개."
        }
        "Hooks" => "훅",
        "invalid" => "유효하지 않음",
        "Saved." => "저장했습니다.",
        "Saved {name}." => "{name}을(를) 저장했습니다.",
        "No change." => "변경 사항이 없습니다.",
        "Your text is kept in {path}." => "작성한 내용은 {path}에 보관됩니다.",
        "The user's own document; always in effect." => "사용자 자신의 문서이며 항상 적용됩니다.",
        "No .dimcode/hooks.json in this workspace." => {
            "이 작업 공간에 .dimcode/hooks.json이 없습니다."
        }
        "Owned by plugin {plugin}." => "플러그인 {plugin} 소유입니다.",
        "The user document turns every hook off (disableAllHooks)." => {
            "사용자 문서가 모든 훅을 끕니다(disableAllHooks)."
        }
        "Enter toggle" => "Enter 전환",
        "Ctrl+E edit" => "Ctrl+E 편집",
        "Ctrl+N add" => "Ctrl+N 추가",
        "Ctrl+R reload" => "Ctrl+R 다시 로드",
        "Ctrl+T run" => "Ctrl+T 실행",
        "Ctrl+X remove" => "Ctrl+X 삭제",
        "Tab inspect" => "Tab 상세",
        "Remove {name}? Enter confirm · Esc cancel" => {
            "{name}을(를) 삭제할까요? Enter 확인 · Esc 취소"
        }
        "↑↓ scroll · Esc back" => "↑↓ 스크롤 · Esc 뒤로",
        "Added." => "추가했습니다.",
        "Removed {name}." => "{name}을(를) 삭제했습니다.",
        "Ran {name}." => "{name}을(를) 실행했습니다.",
        "The user document changed since it was read; nothing was written." => {
            "사용자 문서가 읽은 뒤에 변경되었습니다. 아무것도 쓰지 않았습니다."
        }
        "The job changed since it was read; nothing was written." => {
            "작업이 읽은 뒤에 변경되었습니다. 아무것도 쓰지 않았습니다."
        }
        "The jobs changed since they were read; nothing was written." => {
            "작업이 읽은 뒤에 변경되었습니다. 아무것도 쓰지 않았습니다."
        }
        "Your next edit opens the current document." => "다음 편집은 현재 문서를 엽니다.",
        "Your next add reopens your text." => "다음 추가는 작성한 내용을 다시 엽니다.",
        "Scheduler" => "스케줄러",
        "jobs" => "작업",
        "Added {name}." => "{name}을(를) 추가했습니다.",
        "Run {name}: {outcome}." => "{name}을(를) 실행했습니다: {outcome}.",
        "Recent runs of {name}" => "{name}의 최근 실행",
        "No run yet." => "아직 실행 기록이 없습니다.",
        "No next run." => "다음 실행이 없습니다.",
        "Next run {instant}." => "다음 실행 {instant}.",
        "Last run {outcome} at {instant}." => "마지막 실행 {outcome}, {instant}.",
        "every {minutes} minutes" => "{minutes}분마다",
        "at {instant}" => "{instant}에",
        "cron {expression}" => "cron {expression}",
        "cron {expression} in {zone}" => "cron {expression} ({zone})",
        "The job changed since it was read; nothing was removed." => {
            "작업이 읽은 뒤에 변경되었습니다. 아무것도 삭제하지 않았습니다."
        }
        "The Scheduler has no area document." => "스케줄러에는 영역 문서가 없습니다.",
        "No scheduled job has that id." => "해당 id의 예약 작업이 없습니다.",
        "A scheduled job's run takes no input document." => {
            "예약된 작업의 실행은 입력 문서를 받지 않습니다."
        }
        "This daemon cannot manage {area}." => "{area}을(를) 관리할 수 없습니다.",
        "The document deleted {field}, which the owner cannot clear; nothing was written." => {
            "{field}이(가) 문서에서 삭제되었지만 소유자가 지울 수 없습니다. 아무것도 쓰지 않았습니다."
        }
        "The document changes no field; nothing was written." => {
            "문서가 어떤 필드도 바꾸지 않았습니다. 아무것도 쓰지 않았습니다."
        }
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "이 작업 공간의 훅 설정이 읽은 뒤에 변경되었습니다. 아무것도 쓰지 않았습니다."
        }
        "Workflow" => "워크플로",
        "Saved {name}. It stays disabled until you enable it." => {
            "{name}을(를) 저장했습니다. 활성화할 때까지 비활성 상태로 유지됩니다."
        }
        "The definitions changed since they were read; nothing was written." => {
            "정의가 읽은 후 변경되었습니다. 아무것도 쓰지 않았습니다."
        }
        "A definition's name is its key; add it to save it under another name." => {
            "정의의 이름은 그 키입니다. 다른 이름으로 저장하려면 추가하세요."
        }
        "No project definition is named {name}." => "{name}이라는 프로젝트 정의가 없습니다.",
        "No workflow definition is named {name}." => "{name}이라는 워크플로 정의가 없습니다.",
        "Saved in the user workflow source; always enabled." => {
            "사용자 워크플로 소스에 저장됩니다. 항상 활성화됩니다."
        }
        "Starts only after you enable it." => "활성화한 후에만 시작됩니다.",
        "Defined by package {package}." => "패키지 {package}에서 정의합니다.",
        "{name} — {description} (required)" => "{name} — {description} (필수)",
        "{name} — {description}" => "{name} — {description}",
        "{name} — required" => "{name} — 필수",
        "{name} — default: {default}" => "{name} — 기본값: {default}",
        "and {count} more" => "외 {count}개",
        "Recent runs" => "최근 실행",
        "A workflow definition is saved as an object with a source and a document." => {
            "워크플로 정의는 소스와 문서를 가진 객체로 저장합니다."
        }
        "A definition is saved to the user source or the project source." => {
            "정의는 사용자 소스 또는 프로젝트 소스에 저장합니다."
        }
        "Started {name}: {run_id}." => "{name} 시작됨: {run_id}.",
        "Parameters are a JSON object of strings." => {
            "매개변수는 문자열로 이루어진 JSON 객체입니다."
        }
        _ => return None,
    })
}
