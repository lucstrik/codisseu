//! Full-query CSV export: one row at a time, private temporary file, no clobber.
use crate::remote::Cell;
use std::{
    io::{BufWriter, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

#[derive(Clone, Debug)]
pub struct CsvExportOptions {
    pub header: bool,
    pub delimiter: u8,
    pub null_marker: String,
    pub spreadsheet_safe: bool,
}
impl Default for CsvExportOptions {
    fn default() -> Self {
        Self {
            header: true,
            delimiter: b',',
            null_marker: r"\N".into(),
            spreadsheet_safe: false,
        }
    }
}
#[derive(Clone, Debug)]
pub struct QueryExportRequest {
    /// Reexecutes this captured statement; never removes an explicit LIMIT.
    pub sql: String,
    pub path: PathBuf,
    pub options: CsvExportOptions,
    pub timeout_secs: u64,
}
impl QueryExportRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.sql.trim().is_empty() || self.sql.len() > crate::SQL_BYTE_LIMIT {
            return Err("Exportação exige uma consulta de até 256 KiB".into());
        }
        if !(1..=3600).contains(&self.timeout_secs) {
            return Err("Prazo da exportação deve ser de 1 a 3600 segundos".into());
        }
        if !matches!(self.options.delimiter, b',' | b';' | b'\t' | b'|') {
            return Err("Separador CSV deve ser vírgula, ponto e vírgula, tab ou pipe".into());
        }
        let marker = &self.options.null_marker;
        if marker.is_empty()
            || marker.len() > 128
            || marker
                .bytes()
                .any(|b| b == self.options.delimiter || matches!(b, b'"' | b'\r' | b'\n' | 0))
        {
            return Err(
                "Marcador NULL deve ser não vazio e não conter aspas, separador ou quebra de linha"
                    .into(),
            );
        }
        if self.path.file_name().is_none() {
            return Err("Escolha um nome de arquivo para exportação".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct QueryExportReport {
    pub path: PathBuf,
    pub rows: u64,
    pub bytes: u64,
    pub elapsed_ms: u128,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueryExportSnapshot {
    pub rows: u64,
    pub bytes: u64,
}
#[derive(Clone, Debug, Default)]
pub struct QueryExportProgress {
    rows: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
}
impl QueryExportProgress {
    pub fn snapshot(&self) -> QueryExportSnapshot {
        QueryExportSnapshot {
            rows: self.rows.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
        }
    }
    pub(crate) fn reset(&self) {
        self.publish(0, 0);
    }
    fn publish(&self, rows: u64, bytes: u64) {
        self.bytes.store(bytes, Ordering::Relaxed);
        self.rows.store(rows, Ordering::Relaxed);
    }
}
/// No retained result collection: at most one decoded row plus a 256 KiB write buffer.
pub(crate) struct CsvStreamWriter {
    output: BufWriter<tempfile::NamedTempFile>,
    path: PathBuf,
    options: CsvExportOptions,
    width: usize,
    rows: u64,
    bytes: u64,
    started: Instant,
    progress: QueryExportProgress,
}
impl CsvStreamWriter {
    pub(crate) fn new(
        request: &QueryExportRequest,
        columns: &[String],
        progress: QueryExportProgress,
    ) -> Result<Self, String> {
        request.validate()?;
        if columns.is_empty()
            || columns.len() > 256
            || columns.iter().map(String::len).sum::<usize>() > 1024 * 1024
        {
            return Err("Exportação exige 1–256 colunas e até 1 MiB de metadados".into());
        }
        // symlink_metadata also rejects dangling symlinks; persist_noclobber protects the final race.
        match std::fs::symlink_metadata(&request.path) {
            Ok(_) => return Err("O arquivo de destino já existe; escolha outro nome".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        let parent = request
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        let temp = tempfile::Builder::new()
            .prefix(".codisseu-export-")
            .tempfile_in(parent)
            .map_err(|e| e.to_string())?;
        let mut writer = Self {
            output: BufWriter::with_capacity(256 * 1024, temp),
            path: request.path.clone(),
            options: request.options.clone(),
            width: columns.len(),
            rows: 0,
            bytes: 0,
            started: Instant::now(),
            progress,
        };
        if writer.options.header {
            for (i, column) in columns.iter().enumerate() {
                if i > 0 {
                    writer.write(&[writer.options.delimiter])?;
                }
                writer.field(column, true)?;
            }
            writer.write(b"\r\n")?;
        }
        Ok(writer)
    }
    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        self.output
            .write_all(data)
            .map_err(|e| format!("Falha ao gravar CSV: {e}"))?;
        self.bytes += data.len() as u64;
        Ok(())
    }
    fn field(&mut self, text: &str, formula_safe: bool) -> Result<(), String> {
        let formula = formula_safe
            && self.options.spreadsheet_safe
            && (text.starts_with(['\t', '\r'])
                || text.trim_start().starts_with(['=', '+', '-', '@']));
        let quoted = text == self.options.null_marker
            || formula
            || text
                .bytes()
                .any(|b| b == self.options.delimiter || matches!(b, b'"' | b'\r' | b'\n'));
        if quoted {
            self.write(b"\"")?;
        }
        if formula {
            self.write(b"'")?;
        }
        for (i, part) in text.split('"').enumerate() {
            if i > 0 {
                self.write(b"\"\"")?;
            }
            self.write(part.as_bytes())?;
        }
        if quoted {
            self.write(b"\"")?;
        }
        Ok(())
    }
    pub(crate) fn row(&mut self, cells: &[Cell]) -> Result<(), String> {
        if cells.len() != self.width
            || cells
                .iter()
                .any(|c| crate::transfer::cell_size(c) > crate::CELL_BYTE_LIMIT)
            || cells.iter().map(crate::transfer::cell_size).sum::<usize>()
                > crate::RESULT_BYTE_LIMIT
        {
            return Err("Exportação interrompida: linha inválida, célula acima de 64 KiB ou linha acima de 4 MiB; nenhum arquivo final publicado".into());
        }
        for (i, cell) in cells.iter().enumerate() {
            if i > 0 {
                self.write(&[self.options.delimiter])?;
            }
            match cell {
                Cell::Null => self.write(self.options.null_marker.clone().as_bytes())?,
                Cell::Text(t) | Cell::Json(t) => self.field(t, true)?,
                Cell::Number(t) => self.field(t, false)?,
                Cell::Bool(v) => self.field(if *v { "true" } else { "false" }, false)?,
                Cell::Binary(b) => self.field(&crate::transfer::hex(b), false)?,
            }
        }
        self.write(b"\r\n")?;
        self.rows += 1;
        if self.rows.is_multiple_of(1024) {
            self.progress.publish(self.rows, self.bytes);
        }
        Ok(())
    }
    pub(crate) fn finish(mut self) -> Result<QueryExportReport, String> {
        self.output.flush().map_err(|e| e.to_string())?;
        self.output
            .get_ref()
            .as_file()
            .sync_all()
            .map_err(|e| e.to_string())?;
        let temp = self.output.into_inner().map_err(|e| e.to_string())?;
        temp.persist_noclobber(&self.path)
            .map_err(|e| format!("Destino preservado; CSV não publicado: {}", e.error))?;
        self.progress.publish(self.rows, self.bytes);
        Ok(QueryExportReport {
            path: self.path,
            rows: self.rows,
            bytes: self.bytes,
            elapsed_ms: self.started.elapsed().as_millis(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(path: PathBuf) -> QueryExportRequest {
        QueryExportRequest {
            sql: "SELECT 1".into(),
            path,
            options: CsvExportOptions::default(),
            timeout_secs: 30,
        }
    }
    #[test]
    fn destination_created_during_export_is_preserved_and_temp_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("race.csv");
        let mut writer = CsvStreamWriter::new(
            &request(path.clone()),
            &["n".into()],
            QueryExportProgress::default(),
        )
        .unwrap();
        writer.row(&[Cell::Number("1".into())]).unwrap();
        std::fs::write(&path, "created by another process").unwrap();
        assert!(writer.finish().is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "created by another process"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn dropped_writer_removes_temp_and_empty_query_can_publish_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.csv");
        let writer = CsvStreamWriter::new(
            &request(path.clone()),
            &["n".into()],
            QueryExportProgress::default(),
        )
        .unwrap();
        drop(writer);
        assert!(!path.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        let writer = CsvStreamWriter::new(
            &request(path.clone()),
            &["n".into()],
            QueryExportProgress::default(),
        )
        .unwrap();
        assert_eq!(writer.finish().unwrap().rows, 0);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "n\r\n");
    }
    #[cfg(unix)]
    #[test]
    fn dangling_symlink_is_rejected_without_following_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("link.csv");
        let missing = dir.path().join("missing");
        std::os::unix::fs::symlink(&missing, &path).unwrap();
        assert!(
            CsvStreamWriter::new(
                &request(path.clone()),
                &["n".into()],
                QueryExportProgress::default()
            )
            .is_err()
        );
        assert!(!missing.exists());
        assert!(
            std::fs::symlink_metadata(path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}
