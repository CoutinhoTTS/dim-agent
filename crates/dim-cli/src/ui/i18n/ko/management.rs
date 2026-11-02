pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Loading..." => "불러오는 중…",
        "No entries." => "항목이 없습니다.",
        "No matching entry." => "일치하는 항목이 없습니다.",
        "Issues" => "문제",
        "sources" => "소스",
        "skills" => "스킬",
        "hooks" => "훅",
        "scheduled jobs" => "예약된 작업",
        "models" => "모델",
        "command rules" => "명령 규칙",
        "enabled" => "활성",
        "shadowed by {name}" => "{name}에 의해 가려짐",
        "Reloaded." => "다시 로드했습니다.",
        "Enabled {name}." => "{name}을(를) 활성화했습니다.",
        "Disabled {name}." => "{name}을(를) 비활성화했습니다.",
        "Reloaded {area}: {entries} entries, {disabled} disabled." => {
            "{area} 다시 로드됨: 항목 {entries}개, 비활성 {disabled}개."
        }
        "Reloaded {area}: {entries} entries." => "{area} 다시 로드됨: 항목 {entries}개.",
        "Hooks" => "훅",
        "invalid" => "유효하지 않음",
        "Saved." => "저장했습니다.",
        "Saved {name}." => "{name}을(를) 저장했습니다.",
        "No change." => "변경 사항이 없습니다.",
        "Your text is kept in {path}." => "작성한 내용은 {path}에 보관됩니다.",
        "The user's own document; always in effect." => "사용자 자신의 문서이며 항상 적용됩니다.",
        "There is no hooks file there." => "거기에 훅 파일이 없습니다.",
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
        "unavailable" => "사용 불가",
        "Reset {name}? Enter confirm · Esc cancel" => {
            "{name}을(를) 초기화할까요? Enter 확인 · Esc 취소"
        }
        "Reset {name} to its catalog entry." => "{name}을(를) 카탈로그 항목으로 초기화했습니다.",
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
        "This daemon cannot manage {area}." => "{area}을(를) 관리할 수 없습니다.",
        "This daemon lists {area} but cannot toggle them." => {
            "{area} 목록은 표시할 수 있지만 전환할 수 없습니다."
        }
        "This daemon lists {area} but cannot edit them." => {
            "{area} 목록은 표시할 수 있지만 편집할 수 없습니다."
        }
        "This daemon lists {area} but cannot reload them." => {
            "{area} 목록은 표시할 수 있지만 다시 로드할 수 없습니다."
        }
        "This daemon lists {area} but cannot add them." => {
            "{area} 목록은 표시할 수 있지만 추가할 수 없습니다."
        }
        "This daemon lists {area} but cannot remove them." => {
            "{area} 목록은 표시할 수 있지만 삭제할 수 없습니다."
        }
        "This daemon lists {area} but cannot run them." => {
            "{area} 목록은 표시할 수 있지만 실행할 수 없습니다."
        }
        "This daemon lists {area} but cannot inspect them." => {
            "{area} 목록은 표시할 수 있지만 세부 정보나 기록을 볼 수 없습니다."
        }
        "The document deleted {field}, which the owner cannot clear; nothing was written." => {
            "{field}이(가) 문서에서 삭제되었지만 소유자가 지울 수 없습니다. 아무것도 쓰지 않았습니다."
        }
        "The document changes no field; nothing was written." => {
            "문서가 어떤 필드도 바꾸지 않았습니다. 아무것도 쓰지 않았습니다."
        }
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "이 작업 공간의 훅 설정이 읽은 뒤에 변경되었습니다. 아무것도 쓰지 않았습니다."
        }
        "Models" => "모델",
        "catalog" => "카탈로그",
        "overrides" => "재정의",
        "added" => "추가",
        "Models · {name}" => "모델 · {name}",
        "undeclared" => "선언되지 않음",
        "Context window {window} · max output {output}" => {
            "컨텍스트 창 {window} · 최대 출력 {output}"
        }
        "Endpoint {endpoint}" => "엔드포인트 {endpoint}",
        "Endpoint {endpoint} · override {value}" => "엔드포인트 {endpoint} · 재정의 {value}",
        "The provider reports it unavailable." => "공급자가 사용할 수 없다고 보고합니다.",
        "Limits undeclared; edit the model to declare them." => {
            "한도가 선언되지 않았습니다. 모델을 편집해 선언하세요."
        }
        "Sign in to the account to use its models." => {
            "계정에 로그인하면 모델을 사용할 수 있습니다."
        }
        "The signed-in account changed; reload to list its models." => {
            "로그인한 계정이 바뀌었습니다. 다시 로드해 모델을 나열하세요."
        }
        "The provider has no catalog yet." => "이 공급자에는 아직 카탈로그가 없습니다.",
        "The provider listed no models." => "공급자가 모델을 나열하지 않았습니다.",
        "Listed {count} models." => "모델 {count}개를 나열했습니다.",
        "Refreshed the models." => "모델을 새로 고쳤습니다.",
        "Could not list models: {error}. Showing the stored list." => {
            "모델을 나열할 수 없습니다: {error}. 저장된 목록을 표시합니다."
        }
        "The signed-in account changed; the refresh cleared preferences, overrides and disables." => {
            "로그인한 계정이 바뀌었습니다. 새로 고침이 기본 설정, 재정의, 비활성화를 지웠습니다."
        }
        "The models changed since they were read; nothing was removed." => {
            "모델이 읽은 뒤 변경되었습니다. 이번에는 아무것도 제거하지 않았습니다."
        }
        "The models changed since they were read; nothing was written." => {
            "모델이 읽은 뒤 변경되었습니다. 이번에는 아무것도 쓰지 않았습니다."
        }
        "No provider has that id." => "해당 id의 공급자가 없습니다.",
        "{name} is unavailable ({reason}); its enablement is kept as it is." => {
            "{name}은(는) 사용할 수 없습니다({reason}). 활성화 상태는 그대로 유지됩니다."
        }
        "No model has that id." => "해당 id의 모델이 없습니다.",
        "This model cannot be removed." => "이 모델은 제거할 수 없습니다.",
        "A Provider's models have no area document." => "공급자의 모델에는 영역 문서가 없습니다.",
        "Models have no run operation." => "모델에는 실행 작업이 없습니다.",
        "The document is not an object; nothing was written." => {
            "문서가 객체가 아닙니다. 이번에는 아무것도 쓰지 않았습니다."
        }
        "The document changes no editable field; nothing was written." => {
            "문서가 편집 가능한 필드를 변경하지 않았습니다. 이번에는 아무것도 쓰지 않았습니다."
        }
        "The model document names no model." => "모델 문서가 모델을 지정하지 않습니다.",
        "The provider has no model catalog to write." => {
            "이 공급자에는 쓸 수 있는 모델 카탈로그가 없습니다."
        }
        "ID {id}" => "ID {id}",
        "Source {source}" => "소스 {source}",
        "Status {status}" => "상태 {status}",
        "Tools {value}" => "도구 {value}",
        "Vision {value}" => "비전 {value}",
        "Reasoning {value}" => "추론 {value}",
        "Modalities {list}" => "모달리티 {list}",
        "Description {text}" => "설명 {text}",
        "Release date {date}" => "출시일 {date}",
        "Pricing {value}" => "가격 {value}",
        "Rate limit {value}" => "속도 제한 {value}",
        "Matched provider {id}" => "일치 공급자 {id}",
        "Matched model {id}" => "일치 모델 {id}",
        "Match strategy {strategy}" => "일치 전략 {strategy}",
        "Capability overrides {fields}" => "기능 재정의 {fields}",
        "yes" => "예",
        "no" => "아니요",
        "not supported" => "지원되지 않음",
        "supported" => "지원됨",

        "MCP" => "MCP",
        "MCP servers" => "MCP 서버",
        "Ctrl+T test" => "Ctrl+T 테스트",
        "sign-in required" => "로그인 필요",
        "signed in" => "로그인됨",
        "sign-in expired" => "로그인 만료",
        "missing scopes: {scopes}" => "누락된 범위: {scopes}",
        "Executable: {command}" => "실행 파일: {command}",
        "Stored arguments (hidden): {count}" => "저장된 인수(숨김): {count}",
        "Working directory: {cwd}" => "작업 디렉터리: {cwd}",
        "Environment names: {names}" => "환경 변수 이름: {names}",
        "Endpoint: {endpoint}" => "엔드포인트: {endpoint}",
        "Header names: {names}" => "헤더 이름: {names}",
        "Bearer token: {state}" => "Bearer 토큰: {state}",
        "OAuth: configured (client ID {id})" => "OAuth: 구성됨 (클라이언트 ID {id})",
        "OAuth: configured" => "OAuth: 구성됨",
        "OAuth: not configured" => "OAuth: 구성되지 않음",
        "Metadata URL: {state}" => "메타데이터 URL: {state}",
        "configured" => "구성됨",
        "Owned by a plugin; change it through Plugins." => {
            "플러그인이 소유합니다. 플러그인에서 변경하세요."
        }
        "MCP entry {id}" => "MCP 항목 {id}",
        "Connection {number} · {status}" => "연결 {number} · {status}",
        "Tools have not been discovered." => "도구가 아직 발견되지 않았습니다.",
        "No live session connections." => "활성 세션 연결이 없습니다.",
        "Tools of {id}" => "{id}의 도구",
        "The server exposes no tools." => "서버가 도구를 제공하지 않습니다.",
        "The add document needs an id." => "추가 문서에는 id가 필요합니다.",
        "The add document needs a source of user or project." => {
            "추가 문서의 source는 user 또는 project여야 합니다."
        }
        "The add document needs a configuration object." => {
            "추가 문서에는 configuration 객체가 필요합니다."
        }
        "{id} is already in mcp list; remove it first or choose another id." => {
            "{id}은(는) 이미 mcp list에 있습니다. 먼저 제거하거나 다른 id를 선택하세요."
        }
        "{id} is already in mcp list; choose another id, or change it through its plugin." => {
            "{id}은(는) 이미 mcp list에 있습니다. 다른 id를 선택하거나 플러그인에서 변경하세요."
        }
        "This entry's settings changed since they were read; nothing was written." => {
            "이 항목의 설정이 읽은 후 변경되었습니다. 아무것도 기록되지 않았습니다."
        }
        "The MCP entry changed since it was read; nothing was written." => {
            "MCP 항목이 읽은 후 변경되었습니다. 아무것도 기록되지 않았습니다."
        }
        "The MCP entries changed since they were read; nothing was written." => {
            "MCP 항목이 읽은 후 변경되었습니다. 아무것도 기록되지 않았습니다."
        }
        "The MCP entry changed since it was read; nothing was removed." => {
            "MCP 항목이 읽은 후 변경되었습니다. 아무것도 제거되지 않았습니다."
        }
        "MCP's documents belong to one entry; this save names none." => {
            "MCP 문서는 항목별입니다. 이 저장은 항목을 지정하지 않았습니다."
        }
        "No MCP server has that id." => "해당 id의 MCP 서버가 없습니다.",
        "This workspace has no project MCP source." => {
            "이 작업 공간에는 프로젝트 MCP 소스가 없습니다."
        }
        "Enabled {id} ({source})." => "{id}({source})을(를) 활성화했습니다.",
        "Disabled {id} ({source})." => "{id}({source})을(를) 비활성화했습니다.",
        "It runs this workspace's .mcp.json entry, including later edits to it, in active and new sessions." => {
            "활성 및 새 세션에서 이후 수정을 포함해 이 작업 공간의 .mcp.json 항목을 실행합니다."
        }
        "{id} is not in mcp list; withdrew any stored enablement of it in this workspace." => {
            "{id}은(는) mcp list에 없습니다. 이 작업 공간에 저장된 활성화를 철회했습니다."
        }
        "The MCP catalog could not be read ({error}); withdrew this workspace's enablement of {id}. A user entry with this ID is unchanged." => {
            "MCP 카탈로그를 읽을 수 없습니다({error}). 이 작업 공간에서 {id}의 활성화를 철회했습니다. 같은 ID의 사용자 항목은 변경되지 않았습니다."
        }
        "MCP configuration: {error}" => "MCP 구성: {error}",
        "The server rejected the stored credential; sign in again with mcp login." => {
            "서버가 저장된 자격 증명을 거부했습니다. mcp login으로 다시 로그인하세요."
        }
        "The server requires additional scopes: {scopes}." => {
            "서버가 추가 범위를 요구합니다: {scopes}."
        }
        "The server requires authorization; run mcp login ID." => {
            "서버가 인증을 요구합니다. mcp login ID를 실행하세요."
        }
        "The probe ended: {status}." => "프로브가 종료되었습니다: {status}.",

        "Command rules" => "명령 규칙",
        "user" => "사용자",
        "Matches only this command." => "이 명령만 일치합니다.",
        "Matches commands whose first words are these." => {
            "앞부분이 이 단어들인 명령과 일치합니다."
        }
        "Allows a simple command that would otherwise ask." => {
            "원래 확인을 요청하는 단순 명령을 허용합니다."
        }
        "Denies the command under every preset." => "모든 프리셋에서 이 명령을 거부합니다.",
        "Applies in every Session, in every workspace." => {
            "모든 세션, 모든 워크스페이스에서 적용됩니다."
        }
        "Rule added: {rule}." => "규칙을 추가했습니다: {rule}.",
        "Rule already present: {rule}." => "규칙이 이미 있습니다: {rule}.",
        "Rule removed: {rule}." => "규칙을 삭제했습니다: {rule}.",
        "That command rule is no longer listed; nothing was removed." => {
            "해당 명령 규칙이 더 이상 목록에 없습니다. 아무것도 삭제되지 않았습니다."
        }
        "Command rules changed since they were read; nothing was removed." => {
            "명령 규칙이 읽은 뒤 변경되었습니다. 아무것도 삭제되지 않았습니다."
        }
        "A command rule is DECISION MATCH COMMAND, such as deny prefix git push." => {
            "명령 규칙은 DECISION MATCH COMMAND 형식입니다. 예: deny prefix git push."
        }
        "Command: {command}" => "명령: {command}",
        "Use the line prompt or dimcode permissions." => {
            "줄 프롬프트 또는 dimcode permissions에서 사용하세요."
        }
        _ => return None,
    })
}
