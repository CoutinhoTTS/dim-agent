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
        _ => return None,
    })
}
