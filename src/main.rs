use std::fs::File;
use std::sync::Arc;
use arrow::record_batch::RecordBatch;
use arrow::ipc::writer::StreamWriter;
use arrow::ipc::reader::FileReader;
use datafusion::prelude::ParquetReadOptions;
use datafusion::config::ConfigOptions;
use datafusion::datasource::MemTable;
use datafusion::execution::context::{SessionConfig, SessionContext};

pub fn write_arrow_ipc(batches: &[RecordBatch]) -> Vec<u8> {
    assert!(!batches.is_empty());

    let schema = batches[0].schema();
    let mut buffer = Vec::new();

    {
        let mut writer = StreamWriter::try_new(&mut buffer, &schema).unwrap();

        for batch in batches {
            writer.write(batch).unwrap();
        }

        writer.finish().unwrap();
    }

    buffer
}

#[tokio::main]
async fn main() {
    let mut args: Vec<String> = std::env::args().collect();
    let file = args.pop().expect("expected a data file");

    let mut options = ConfigOptions::new();
    options.execution.parquet.pushdown_filters = true;
    options.execution.parquet.reorder_filters = true;
    options.optimizer.repartition_sorts = true;

    let config = SessionConfig::from(options)
        .with_batch_size(8 * 1024)
        .with_target_partitions(8)
        .with_repartition_joins(true)
        .with_repartition_aggregations(true)
        .with_repartition_sorts(true)
        .with_repartition_windows(true);

    let ctx = SessionContext::with_config(config);

    if file.ends_with("parquet") {
        ctx.register_parquet("t0", &file, ParquetReadOptions::default()).await.unwrap();
    } else if file.ends_with("arrow") {
        let file = File::open(file).unwrap();
        let reader = FileReader::try_new(file, None).unwrap();
        let batches = reader.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
        let schema = batches[0].schema();
        let table = Arc::new(MemTable::try_new(schema, vec![batches]).unwrap());
        ctx.register_table("t0", table).unwrap();
    } else {
        panic!("unsupported file type");
    }

    const QUERY: &str = r#"SELECT "ap_name" AS "ap_name_inner_alias", count(distinct "mac")  AS  "daily_unique" FROM t0 GROUP BY "ap_name", date_trunc('day', timestamp);"#;
    let df = ctx.sql(QUERY).await.unwrap();
    let results: Vec<RecordBatch> = df.collect().await.unwrap();
    let serialized_batches = write_arrow_ipc(&results);
    std::fs::write("output.arrow", serialized_batches).unwrap();
}
