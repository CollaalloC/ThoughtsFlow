use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};

use uuid::Uuid;

use crate::ports::{DecisionPacketExport, DecisionPacketWriteError, DecisionPacketWriter};

#[derive(Clone, Debug)]
pub struct LocalDecisionPacketWriter {
    export_root: PathBuf,
}

impl LocalDecisionPacketWriter {
    pub fn new(export_root: PathBuf) -> Self {
        Self { export_root }
    }
}

impl DecisionPacketWriter for LocalDecisionPacketWriter {
    fn write(
        &self,
        _workspace_id: &str,
        markdown: &str,
    ) -> Result<DecisionPacketExport, DecisionPacketWriteError> {
        fs::create_dir_all(&self.export_root)
            .map_err(|error| DecisionPacketWriteError(error.to_string()))?;
        let destination = self
            .export_root
            .join(format!("decision-packet-{}.md", Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(|error| DecisionPacketWriteError(error.to_string()))?;
        file.write_all(markdown.as_bytes())
            .map_err(|error| DecisionPacketWriteError(error.to_string()))?;
        file.sync_all()
            .map_err(|error| DecisionPacketWriteError(error.to_string()))?;
        Ok(DecisionPacketExport {
            path: destination.to_string_lossy().into_owned(),
            bytes_written: markdown.len() as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::LocalDecisionPacketWriter;
    use crate::ports::DecisionPacketWriter;

    #[test]
    fn decision_packets_are_created_inside_the_export_root_without_overwrite() {
        let root = tempfile::tempdir().unwrap();
        let writer = LocalDecisionPacketWriter::new(root.path().to_path_buf());

        let first = writer
            .write("workspace/../../escape", "first packet")
            .unwrap();
        let second = writer
            .write("workspace/../../escape", "second packet")
            .unwrap();

        assert_ne!(first.path, second.path);
        assert!(std::path::Path::new(&first.path).starts_with(root.path()));
        assert!(std::path::Path::new(&second.path).starts_with(root.path()));
        assert_eq!(std::fs::read_to_string(first.path).unwrap(), "first packet");
        assert_eq!(
            std::fs::read_to_string(second.path).unwrap(),
            "second packet"
        );
    }
}
