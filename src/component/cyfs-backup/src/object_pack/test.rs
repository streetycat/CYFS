use super::pack::*;
use cyfs_base::*;
use cyfs_core::*;

use async_std::io::ReadExt;
use std::collections::HashSet;

async fn test_pack() {
    let count: usize = 1024 * 10;
    let file_buffer: Vec<u8> = (0..1024 * 4).map(|_| rand::random::<u8>()).collect();

    let path = cyfs_util::get_temp_path().join("test_pack");
    if !path.is_dir() {
        std::fs::create_dir_all(&path).unwrap();
    }

    let backup_file = path.join("backup.zip");

    let mut pack = ObjectPackFactory::create_zip_writer(backup_file.clone());
    pack.open().await.unwrap();

    for i in 0..count {
        let obj = Text::create(&format!("test{}", i), "", "");
        let id = obj.desc().calculate_id();

        let data = async_std::io::Cursor::new(file_buffer.clone());
        pack.add_data(&id, Box::new(data)).await.unwrap();

        if i % 1024 == 0 {
            info!("gen dir index: {}", i);
            // async_std::task::sleep(std::time::Duration::from_secs(5)).await;

            let len = pack.flush().await.unwrap();
            info!("pack file len: {}", len);
        }
    }

    pack.finish().await.unwrap();

    let mut pack_reader = ObjectPackFactory::create_zip_reader(backup_file);
    pack_reader.open().await.unwrap();

    let mut all = HashSet::new();
    for i in 0..count {
        let obj = Text::create(&format!("test{}", i), "", "");
        let id = obj.desc().calculate_id();

        all.insert(id.clone());

        let mut data = pack_reader.get_data(&id).await.unwrap().unwrap();
        let mut buf = vec![];
        data.read_to_end(&mut buf).await.unwrap();
        assert_eq!(buf, file_buffer);
    }

    pack_reader.reset().await;
    loop {
        let ret = pack_reader.next_data().await.unwrap();
        if ret.is_none() {
            break;
        }

        let (object_id, mut data) = ret.unwrap();
        assert!(all.remove(&object_id));

        let mut buf = vec![];
        data.read_to_end(&mut buf).await.unwrap();
        assert_eq!(buf, file_buffer);
    }

    assert!(all.is_empty());
}

#[test]
fn test() {
    async_std::task::block_on(test_pack());
}
