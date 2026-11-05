pub struct Phrases<'a> {
    pub name: &'a str,
}

impl Phrases<'_> {
    pub fn help(&self) -> String {
        format!(
            "Я {}. Управляю музыкой на вашем компьютере. Скажите: следующий трек, пауза, включи Земфиру, сделай погромче.",
            self.name
        )
    }

    pub fn hello(&self) -> String {
        format!("Привет, я {}. Что включить?", self.name)
    }

    pub fn link_needed(&self) -> &'static str {
        "Чтобы управлять музыкой, свяжите аккаунт."
    }

    pub fn offline(&self) -> &'static str {
        "Компьютер не отвечает. Откройте SoundCloud Desktop."
    }

    pub fn star_needed(&self) -> &'static str {
        "Нужна подписка STAR. Оформите её в приложении."
    }

    pub fn too_fast(&self) -> &'static str {
        "Слишком много команд. Подождите немного."
    }

    pub fn repeat(&self) -> &'static str {
        "Повторите, пожалуйста."
    }

    pub fn done(&self) -> &'static str {
        "Готово."
    }

    pub fn failed(&self) -> &'static str {
        "Не получилось."
    }

    pub fn busy(&self) -> &'static str {
        "Сервис занят. Попробуйте ещё раз."
    }

    pub fn say_code(&self) -> &'static str {
        "Чтобы связать аккаунт, скажите шесть цифр кода из приложения на компьютере."
    }

    pub fn code_needs_six(&self) -> &'static str {
        "Нужно ровно шесть цифр. Назовите код ещё раз."
    }

    pub fn code_wrong(&self) -> &'static str {
        "Код не подошёл. Назовите шесть цифр из приложения ещё раз."
    }

    pub fn code_throttled(&self) -> &'static str {
        "Слишком много попыток. Подождите минуту."
    }

    pub fn confirm_on_pc(&self) -> &'static str {
        "Подтвердите на компьютере."
    }

    pub fn confirm_with_hint(&self, hint: &str) -> String {
        format!("Подтвердите на компьютере. Там будет число {hint}.")
    }

    pub fn link_denied(&self) -> &'static str {
        "Привязка отклонена на компьютере."
    }

    pub fn link_silent(&self) -> &'static str {
        "Компьютер не ответил. Назовите код ещё раз, когда откроете приложение."
    }

    pub fn link_offline(&self) -> &'static str {
        "Компьютер не отвечает. Откройте SoundCloud Desktop и назовите код ещё раз."
    }

    pub fn code_capped(&self) -> &'static str {
        "На сегодня слишком много запросов на подтверждение. Попробуйте завтра."
    }

    pub fn linked(&self) -> &'static str {
        "Готово, аккаунт связан. Что включить?"
    }
}

pub fn wants_help(utterance: &str) -> bool {
    let plain: String = utterance
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let plain = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    if plain.chars().count() > 40 {
        return false;
    }
    [
        "помощь",
        "справка",
        "помоги",
        "что ты умеешь",
        "что ты можешь",
        "что умеешь",
        "что можешь",
    ]
    .iter()
    .any(|key| plain == *key)
        || plain.contains("что ты умеешь")
        || plain.contains("что ты можешь")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_is_recognised_in_common_wordings() {
        for text in [
            "Помощь",
            "что ты умеешь?",
            "Что ты умеешь",
            "что ты можешь",
            "помоги",
            "справка!",
        ] {
            assert!(wants_help(text), "{text}");
        }
    }

    #[test]
    fn commands_are_not_help() {
        for text in [
            "включи следующий трек",
            "поставь помощь которую ты ждёшь на повтор",
            "",
        ] {
            assert!(!wants_help(text), "{text}");
        }
    }

    #[test]
    fn the_name_comes_from_the_caller() {
        let phrases = Phrases { name: "Тест" };
        assert!(phrases.help().starts_with("Я Тест."));
        assert!(phrases.hello().contains("Тест"));
        assert!(phrases.confirm_with_hint("47").contains("число 47"));
    }
}
