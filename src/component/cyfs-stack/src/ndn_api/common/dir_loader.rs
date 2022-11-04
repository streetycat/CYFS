use super::super::data::LocalDataManager;
use cyfs_base::*;
use cyfs_lib::*;

use async_std::io::ReadExt;
use std::borrow::Cow;

pub(crate) struct DirLoader {
    data_manager: LocalDataManager,
}

impl DirLoader {
    pub fn new(data_manager: LocalDataManager) -> Self {
        Self { data_manager }
    }

    pub async fn load_desc_obj_list<'a>(
        &self,
        dir_id: &ObjectId,
        dir: &'a Dir,
    ) -> BuckyResult<Cow<'a, NDNObjectList>> {
        let obj_list = match &dir.desc().content().obj_list() {
            NDNObjectInfo::Chunk(id) => {
                let body = self.load_body_obj_list(dir_id, dir).await?;
                let list = self
                    .load_from_body_and_chunk_manager(dir_id, dir, &id, &body)
                    .await?;
                Cow::Owned(list)
            }
            NDNObjectInfo::ObjList(list) => Cow::Borrowed(list),
        };

        Ok(obj_list)
    }

    pub async fn load_desc_and_body<'a>(
        &self,
        dir_id: &ObjectId,
        dir: &'a Dir,
    ) -> BuckyResult<(
        Cow<'a, NDNObjectList>,
        Option<Cow<'a, DirBodyContentObjectList>>,
    )> {
        let body = self.load_body_obj_list(dir_id, dir).await?;

        let obj_list = match &dir.desc().content().obj_list() {
            NDNObjectInfo::Chunk(id) => {
                let list = self
                    .load_from_body_and_chunk_manager(dir_id, dir, &id, &body)
                    .await?;
                Cow::Owned(list)
            }
            NDNObjectInfo::ObjList(list) => Cow::Borrowed(list),
        };

        Ok((obj_list, body))
    }

    async fn load_from_body_and_chunk_manager<'a, T: for<'de> RawDecode<'de>>(
        &self,
        dir_id: &ObjectId,
        dir: &'a Dir,
        chunk_id: &ChunkId,
        body: &Option<Cow<'a, DirBodyContentObjectList>>,
    ) -> BuckyResult<T> {
        // first try to load chunk from body
        if let Some(body) = body {
            let ret = body.get(chunk_id.as_object_id());
            if ret.is_some() {
                debug!(
                    "load chunk from dir body! dir={}, chunk={}",
                    dir_id, chunk_id
                );
                let buf = ret.unwrap();
                let (ret, _) = T::raw_decode(&buf)?;
                return Ok(ret);
            }
        }

        // then try to load chunk from chunk manager
        self.load_from_chunk_manager(dir_id, chunk_id).await
    }

    async fn load_body_obj_list<'a>(
        &self,
        dir_id: &ObjectId,
        dir: &'a Dir,
    ) -> BuckyResult<Option<Cow<'a, DirBodyContentObjectList>>> {
        let ret = match dir.body() {
            Some(body) => {
                let list = match body.content() {
                    DirBodyContent::Chunk(id) => {
                        let list: DirBodyContentObjectList =
                            self.load_from_chunk_manager(dir_id, id).await?;
                        Cow::Owned(list)
                    }
                    DirBodyContent::ObjList(list) => Cow::Borrowed(list),
                };

                Some(list)
            }
            None => None,
        };

        Ok(ret)
    }

    async fn load_from_chunk_manager<T: for<'a> RawDecode<'a>>(
        &self,
        dir_id: &ObjectId,
        chunk_id: &ChunkId,
    ) -> BuckyResult<T> {
        let ret = self
            .data_manager
            .get_chunk(chunk_id, None)
            .await
            .map_err(|e| {
                error!(
                    "load dir desc chunk error! dir={}, chunk={}, {}",
                    dir_id, chunk_id, e,
                );
                e
            })?;

        if ret.is_none() {
            let msg = format!(
                "load dir desc chunk but not found! dir={}, chunk={}",
                dir_id, chunk_id
            );
            error!("{}", msg);
            return Err(BuckyError::new(BuckyErrorCode::NotFound, msg));
        }

        let (mut reader, len) = ret.unwrap();
        let mut buf = vec![];
        reader.read_to_end(&mut buf).await.map_err(|e| {
            let msg = format!(
                "load dir desc chunk to buf error! dir={}, chunk={}, {}",
                dir_id, chunk_id, e
            );
            error!("{}", msg);
            BuckyError::new(BuckyErrorCode::IoError, msg)
        })?;

        let (ret, _) = T::raw_decode(&buf)?;
        Ok(ret)
    }
}
