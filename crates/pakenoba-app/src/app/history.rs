//! 元に戻す、やり直す。
//!
//! 変える前の状態を積んでおき、戻すときに取り出す。確認のダイアログを出す代わりに、戻せるようにする。

/// 覚えておく数の上限。
const LIMIT: usize = 200;

pub(super) struct History<T> {
    past: Vec<T>,
    future: Vec<T>,
}

impl<T> Default for History<T> {
    fn default() -> Self {
        History { past: Vec::new(), future: Vec::new() }
    }
}

impl<T> History<T> {
    /// 変える前の状態を覚える。新しく変えたら、やり直しの先は捨てる。
    pub fn record(&mut self, before: T) {
        self.past.push(before);
        if self.past.len() > LIMIT {
            self.past.remove(0);
        }
        self.future.clear();
    }

    /// 1 つ前の状態を返す。いまの状態（`current` で作る）は、やり直し用に取っておく。
    pub fn undo(&mut self, current: impl FnOnce() -> T) -> Option<T> {
        let before = self.past.pop()?;
        self.future.push(current());
        Some(before)
    }

    /// 戻したものを、やり直す。
    pub fn redo(&mut self, current: impl FnOnce() -> T) -> Option<T> {
        let after = self.future.pop()?;
        self.past.push(current());
        Some(after)
    }

    /// 元に戻せる数（エージェントの依頼のあとに、ほかの変更がないかを見るため）。
    pub fn depth(&self) -> usize {
        self.past.len()
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 戻して_やり直せる() {
        let mut history = History::default();
        let mut state = 0;
        for next in 1..=3 {
            history.record(state);
            state = next;
        }
        assert!(history.can_undo() && !history.can_redo());
        state = history.undo(|| state).unwrap();
        assert_eq!(state, 2);
        state = history.undo(|| state).unwrap();
        assert_eq!(state, 1);
        assert!(history.can_redo());
        state = history.redo(|| state).unwrap();
        assert_eq!(state, 2);
        state = history.redo(|| state).unwrap();
        assert_eq!(state, 3);
        assert!(history.redo(|| state).is_none());
    }

    #[test]
    fn 戻したあとに変えたら_やり直しの先は捨てる() {
        let mut history = History::default();
        history.record(0);
        history.record(1);
        let state = history.undo(|| 2).unwrap();
        assert_eq!(state, 1);
        history.record(state);
        assert!(!history.can_redo());
        // 戻せないときは、いまの状態を作らない（やり直しにも積まない）。
        let mut empty: History<i32> = History::default();
        assert!(empty.undo(|| panic!("呼ばれない")).is_none());
        assert!(!empty.can_redo());
    }

    #[test]
    fn 覚える数には上限がある() {
        let mut history = History::default();
        for i in 0..LIMIT + 50 {
            history.record(i);
        }
        let mut count = 0;
        let mut last = 0;
        while let Some(state) = history.undo(|| 0) {
            last = state;
            count += 1;
        }
        assert_eq!(count, LIMIT);
        assert_eq!(last, 50, "古いものから捨てる");
    }
}
