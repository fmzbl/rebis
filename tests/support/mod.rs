#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};

use rebis_lang::{ModuleName, ModuleResolver, Oracle, Record};

pub struct ScriptedOracle {
    replies: RefCell<VecDeque<Result<Option<String>, String>>>,
    prompts: RefCell<Vec<String>>,
}

impl Default for ScriptedOracle {
    fn default() -> Self {
        Self::answers(&[])
    }
}

impl ScriptedOracle {
    pub fn answers(answers: &[Option<&str>]) -> Self {
        Self::results(
            answers
                .iter()
                .map(|answer| Ok(answer.map(str::to_string)))
                .collect(),
        )
    }

    pub fn results(replies: Vec<Result<Option<String>, String>>) -> Self {
        Self {
            replies: RefCell::new(replies.into()),
            prompts: RefCell::new(Vec::new()),
        }
    }

    pub fn prompts(&self) -> Vec<String> {
        self.prompts.borrow().clone()
    }

    fn next(&self, prompt: &str) -> Result<Option<String>, String> {
        self.prompts.borrow_mut().push(prompt.to_string());
        self.replies.borrow_mut().pop_front().unwrap_or(Ok(None))
    }
}

impl Oracle for ScriptedOracle {
    fn fire(&self, prompt: &str) -> Option<String> {
        self.next(prompt).ok().flatten()
    }

    fn try_fire(&self, prompt: &str) -> Result<Option<String>, String> {
        self.next(prompt)
    }
}

#[derive(Default)]
pub struct MemoryModules {
    sources: BTreeMap<String, Result<Option<String>, String>>,
    requests: RefCell<Vec<String>>,
}

impl MemoryModules {
    pub fn with(mut self, name: &str, source: &str) -> Self {
        self.sources
            .insert(name.to_string(), Ok(Some(source.to_string())));
        self
    }

    pub fn failing(mut self, name: &str, message: &str) -> Self {
        self.sources
            .insert(name.to_string(), Err(message.to_string()));
        self
    }

    pub fn requests(&self) -> Vec<String> {
        self.requests.borrow().clone()
    }
}

impl ModuleResolver for MemoryModules {
    fn resolve(&self, module: &ModuleName) -> Result<Option<String>, String> {
        self.requests.borrow_mut().push(module.to_string());
        self.sources
            .get(module.as_str())
            .cloned()
            .unwrap_or(Ok(None))
    }
}

pub fn empty_record() -> Record {
    Record::from_texts::<&str>(&[])
}
