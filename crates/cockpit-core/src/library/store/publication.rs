use super::*;

impl Store {
    pub(super) fn prepare_publication(
        &self,
        index: &mut Index,
        entry: &LibraryIndexEntry,
        previous: Option<&str>,
        confirmed: Option<&[LibraryConflictFile]>,
        target_root: &Dir,
        target_name: &str,
    ) -> Result<(Option<LibraryIndexEntry>, Option<Vec<MarkerFile>>), InspectionError> {
        let mut old = index
            .items
            .iter()
            .find(|e| e.summary.item_id == entry.summary.item_id)
            .cloned();
        if index.items.iter().any(|item| {
            item.summary.item_id != entry.summary.item_id
                && item.summary.item_path == entry.summary.item_path
        }) {
            return Err(error(
                "library_conflict",
                "Library destination is already owned by another item",
            ));
        }
        if old.is_none() && exists(target_root, target_name)? {
            return Err(error(
                "library_conflict",
                "Library destination is already occupied",
            ));
        }
        if old.as_ref().map(|e| e.summary.revision.as_str()) != previous {
            return Err(error(
                "library_conflict",
                "Library revision changed before publish",
            ));
        }
        let predecessor = if let Some(old) = &old {
            let snapshot = owned_inventory(&self.item_dir(&old.summary.item_path)?, old)?;
            check_confirmation(&conflicts(old, &snapshot), confirmed)?;
            Some(snapshot)
        } else {
            None
        };
        if old.is_none() && index.items.len() >= self.max_items {
            return Err(error(
                "library_full",
                "Library item limit reached; remove an item before adding another",
            ));
        }
        let mut prospective_index = index.clone();
        upsert(&mut prospective_index, entry.clone());
        prospective_index.generation = Uuid::new_v4().to_string();
        let prospective = serde_json::to_vec_pretty(&prospective_index)
            .map_err(|e| corrupt(e.to_string()))?;
        if prospective.len() as u64 > MAX_INDEX {
            return Err(error(
                "library_full",
                "Library index capacity reached; remove an item before adding another",
            ));
        }
        if let Some(previous_entry) = old.as_ref()
            && previous_entry.summary.item_path != entry.summary.item_path
        {
            if index.items.iter().any(|item| {
                item.summary.item_id != previous_entry.summary.item_id
                    && item.summary.item_path == entry.summary.item_path
            }) || exists(target_root, target_name)?
            {
                return Err(error("library_conflict", "Library destination is already occupied"));
            }
            old = Some(self.move_item(index, previous_entry, &entry.summary.item_path)?);
        }
        Ok((old, predecessor))
    }

    pub(super) fn apply_publication(
        &self,
        stage: &Stage,
        target_root: &Dir,
        target_name: &str,
        intent: &mut Intent,
        index: &mut Index,
    ) -> Result<(), InspectionError> {
        let mut method = intent.method;
        self.fault("journal")?;
        if method == Method::Exchange {
            match rename_special(&self.staging, &stage.name, target_root, target_name, true) {
                Ok(()) => {}
                Err(e) if matches!(e.raw_os_error(), Some(22 | 38 | 95)) => {
                    method = Method::TwoRename;
                    intent.method = method;
                    self.write_intent(intent)?;
                }
                Err(e) => return Err(io_error(e)),
            }
        }
        if method == Method::Merge {
            let target = target_root.open_dir_nofollow(target_name).map_err(io_error)?;
            self.publish_owned_entries(&target, &stage.dir, intent)?;
            self.fault("entry_published")?;
        }
        if method == Method::TwoRename {
            rename_special(
                target_root,
                target_name,
                &self.trash,
                &intent.backup,
                false,
            )
            .map_err(io_error)?;
            sync(target_root)?;
            sync(&self.trash)?;
            self.fault("old_to_backup")?;
        }
        if matches!(method, Method::TwoRename | Method::NewTarget) {
            match rename_special(
                &self.staging,
                &stage.name,
                target_root,
                target_name,
                false,
            ) {
                Ok(()) => {}
                Err(e)
                    if method == Method::NewTarget
                        && (e.kind() == io::ErrorKind::AlreadyExists
                            || e.raw_os_error() == Some(17)) =>
                {
                    self.finish(intent)?;
                    return Err(error(
                        "library_conflict",
                        "Library destination is already occupied",
                    ));
                }
                Err(e) => return Err(io_error(e)),
            }
        }
        self.fault("rename_unsynced")?;
        sync(target_root)?;
        sync(&self.staging)?;
        self.fault("new_to_target")?;
        upsert(index, intent.new_entry.as_ref().unwrap().clone());
        self.commit(index)?;
        self.fault("index_commit")?;
        self.finish(intent)
    }
}
