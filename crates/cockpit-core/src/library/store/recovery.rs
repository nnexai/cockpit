use super::*;

impl Store {
    pub(super) fn recover_intent(
        &self,
        intent: &Intent,
        index: &mut Index,
    ) -> Result<(), InspectionError> {
        let (target_root, target_name) = self.item_parent(&intent.target)?;
        let target = exists(&target_root, target_name)?;
        if intent.method == Method::Move {
            return self.recover_move(intent, index, &target_root, target_name, target);
        }
        let backup = exists(&self.trash, &intent.backup)?;
        let target_new = intent.new_entry.as_ref().is_some_and(|entry| {
            target_root
                .open_dir_nofollow(target_name)
                .ok()
                .is_some_and(|dir| verify_entry(&dir, entry).is_ok())
        });
        let is_old = |parent: &Dir, name: &str| {
            intent.predecessor.as_ref().is_some_and(|snapshot| {
                intent.old_entry.as_ref().is_some_and(|entry| {
                    parent
                        .open_dir_nofollow(name)
                        .ok()
                        .is_some_and(|dir| owned_inventory(&dir, entry).is_ok_and(|actual| actual == *snapshot))
                })
            })
        };
        let target_old = is_old(&target_root, target_name);
        let mut forward = false;
        let mut rollback = false;
        match intent.method {
            Method::Merge if target_old && !backup => rollback = true,
            Method::Merge if target && target_new => forward = true,
            Method::Merge if target => {
                let target_dir = target_root.open_dir_nofollow(target_name).map_err(io_error)?;
                let stage = intent.staging.as_ref().unwrap();
                let stage_dir = self.staging.open_dir_nofollow(stage).map_err(io_error)?;
                self.publish_owned_entries(&target_dir, &stage_dir, intent)?;
                forward = true;
            }
            Method::Remove if target && !backup => rollback = target_old,
            Method::Remove if !target && backup && is_old(&self.trash, &intent.backup) => {
                self.sync_transition(intent)?;
                index.items.retain(|e| e.summary.item_id != intent.item_id);
                self.commit(index)?;
                self.finish(intent)?;
                return Ok(());
            }
            Method::RemoveOwned if target => {
                let target_dir = target_root.open_dir_nofollow(target_name).map_err(io_error)?;
                let backup_dir = open_child(&self.trash, &intent.backup)?;
                let old = intent.old_entry.as_ref().unwrap();
                for name in owned_roots(old) {
                    if exists(&target_dir, &name)? && !exists(&backup_dir, &name)? {
                        rename_special(&target_dir, &name, &backup_dir, &name, false)
                            .map_err(io_error)?;
                    }
                }
                if owned_inventory(&backup_dir, old)? == *intent.predecessor.as_ref().unwrap() {
                    self.sync_transition(intent)?;
                    index.items.retain(|e| e.summary.item_id != intent.item_id);
                    self.commit(index)?;
                    self.finish(intent)?;
                    self.prune_empty(&intent.target)?;
                    return Ok(());
                }
            }
            Method::NewTarget if !target => rollback = true,
            Method::NewTarget if target_new => forward = true,
            Method::Exchange if target_new => {
                forward = intent
                    .staging
                    .as_ref()
                    .is_some_and(|stage| is_old(&self.staging, stage))
            }
            Method::TwoRename if target_new => forward = is_old(&self.trash, &intent.backup),
            Method::Exchange | Method::TwoRename if target_old && !backup => rollback = true,
            Method::TwoRename if !target && backup && is_old(&self.trash, &intent.backup) => {
                let entry = intent.new_entry.as_ref().unwrap();
                let stage = intent.staging.as_ref().unwrap();
                let valid = self
                    .staging
                    .open_dir_nofollow(stage)
                    .ok()
                    .is_some_and(|dir| verify_entry(&dir, entry).is_ok());
                if valid {
                    rename_special(&self.staging, stage, &target_root, target_name, false)
                        .map_err(io_error)?;
                    sync(&self.staging)?;
                    forward = true;
                } else {
                    rename_special(
                        &self.trash,
                        &intent.backup,
                        &target_root,
                        target_name,
                        false,
                    )
                    .map_err(io_error)?;
                    sync(&self.trash)?;
                    rollback = true;
                }
                sync(&target_root)?;
            }
            _ => {}
        }
        self.finish_recovery(intent, index, forward, rollback)
    }

    fn recover_move(
        &self,
        intent: &Intent,
        index: &mut Index,
        target_root: &Dir,
        target_name: &str,
        target: bool,
    ) -> Result<(), InspectionError> {
        let source = intent.source.as_deref().ok_or_else(|| corrupt("move intent has no source"))?;
        let (source_root, source_name) = self.item_parent(source)?;
        let source_exists = exists(&source_root, source_name)?;
        let expected = intent.move_inventory.as_ref().ok_or_else(|| corrupt("move intent has no inventory"))?;
        let at_source = source_exists
            && source_root.open_dir_nofollow(source_name).ok()
                .is_some_and(|dir| inventory(&dir).is_ok_and(|actual| actual == *expected));
        let at_target = target
            && target_root.open_dir_nofollow(target_name).ok()
                .is_some_and(|dir| inventory(&dir).is_ok_and(|actual| actual == *expected));
        if at_source && !target {
            self.sync_transition(intent)?;
            self.finish(intent)?;
            return Ok(());
        }
        if at_target && !source_exists {
            self.sync_transition(intent)?;
            for moved in intent.moved_entries.as_ref().ok_or_else(|| corrupt("move intent has no entries"))? {
                upsert(index, moved.clone());
            }
            self.commit(index)?;
            self.finish(intent)?;
            return Ok(());
        }
        Err(corrupt("move journal does not match a recoverable filesystem state"))
    }

    fn finish_recovery(
        &self,
        intent: &Intent,
        index: &mut Index,
        forward: bool,
        rollback: bool,
    ) -> Result<(), InspectionError> {
        if forward || rollback {
            // A prior process may have died immediately after rename, before its fsync.
            // Sync all rename parents even when recovery only observed the transition.
            self.sync_transition(intent)?;
            if forward {
                upsert(index, intent.new_entry.as_ref().unwrap().clone());
                self.commit(index)?;
            } else if let Some(old) = &intent.old_entry {
                upsert(index, old.clone());
                self.commit(index)?;
            }
            self.finish(intent)?;
        } else {
            if let Some(entry) = index
                .items
                .iter_mut()
                .find(|e| e.summary.item_id == intent.item_id)
            {
                entry.summary.state = LibraryItemState::Failed;
                entry.summary.diagnostics = vec![ProjectDiagnostic {
                    code: "library_corrupt".into(),
                    message:
                        "Journal does not match a recoverable filesystem state; files retained"
                            .into(),
                    path: Some(intent.target.clone()),
                }];
                self.commit(index)?;
            } else {
                return Err(corrupt(
                    "unindexed journal target is not recoverable; files retained",
                ));
            }
        }
        Ok(())
    }
}
