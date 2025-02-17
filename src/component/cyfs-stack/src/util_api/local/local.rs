use super::bdt_access_info::BdtNetworkAccessInfoManager;
use super::dir_helper::*;
use crate::config::StackGlobalConfig;
use crate::resolver::OodResolver;
use crate::sync::DeviceSyncClient;
use crate::util::*;
use crate::zone::*;
use cyfs_base::*;
use cyfs_bdt::StackGuard;
use cyfs_core::ZoneObj;
use cyfs_lib::*;
use cyfs_util::*;

use crate::util_api::local::{
    BuildDirParams, BuildDirTaskFactory, BuildDirTaskStatus, BuildFileParams, BuildFileTaskFactory,
    BuildFileTaskStatus,
};
use once_cell::sync::OnceCell;

use cyfs_task_manager::{TaskManager, BUILD_DIR_TASK, BUILD_FILE_TASK};
use std::sync::Arc;

pub(crate) struct UtilLocalService {
    noc: NamedObjectCacheRef,
    bdt_stack: StackGuard,
    zone_manager: ZoneManagerRef,

    ood_resolver: OodResolver,

    sync_client: Arc<OnceCell<Arc<DeviceSyncClient>>>,

    access_info_manager: BdtNetworkAccessInfoManager,

    task_manager: Arc<TaskManager>,

    config: StackGlobalConfig,
}

impl Clone for UtilLocalService {
    fn clone(&self) -> Self {
        Self {
            noc: self.noc.clone(),
            bdt_stack: self.bdt_stack.clone(),
            zone_manager: self.zone_manager.clone(),
            ood_resolver: self.ood_resolver.clone(),
            sync_client: self.sync_client.clone(),
            access_info_manager: self.access_info_manager.clone(),
            task_manager: self.task_manager.clone(),
            config: self.config.clone(),
        }
    }
}

impl UtilLocalService {
    pub(crate) fn new(
        noc: NamedObjectCacheRef,
        ndc: Box<dyn NamedDataCache>,
        bdt_stack: StackGuard,
        zone_manager: ZoneManagerRef,
        ood_resolver: OodResolver,
        task_manager: Arc<TaskManager>,
        config: StackGlobalConfig,
    ) -> Self {
        let access_info_manager = BdtNetworkAccessInfoManager::new(bdt_stack.clone());

        task_manager
            .register_task_factory(BuildFileTaskFactory::new(noc.clone(), ndc))
            .unwrap();
        task_manager
            .register_task_factory(BuildDirTaskFactory::new(
                Arc::downgrade(&task_manager),
                noc.clone(),
            ))
            .unwrap();

        Self {
            noc,
            bdt_stack,
            zone_manager,
            ood_resolver,
            sync_client: Arc::new(OnceCell::new()),
            access_info_manager,
            task_manager,
            config,
        }
    }

    pub fn clone_processor(&self) -> UtilInputProcessorRef {
        Arc::new(Box::new(self.clone()))
    }

    pub(crate) fn bind_sync_client(&self, sync_client: Arc<DeviceSyncClient>) {
        if let Err(_) = self.sync_client.set(sync_client) {
            unreachable!();
        }
    }

    async fn get_device(
        &self,
        _req: UtilGetDeviceInputRequest,
    ) -> BuckyResult<UtilGetDeviceInputResponse> {
        let resp = UtilGetDeviceInputResponse {
            device_id: self.bdt_stack.local_device_id().to_owned(),
            device: self.bdt_stack.sn_client().ping().default_local(),
        };

        Ok(resp)
    }

    async fn get_zone(
        &self,
        req: UtilGetZoneInputRequest,
    ) -> BuckyResult<UtilGetZoneInputResponse> {
        let resp = match req.object_id {
            Some(object_id) => {
                let obj_type = object_id.obj_type_code();

                let zone = match obj_type {
                    ObjectTypeCode::Device
                    | ObjectTypeCode::People
                    | ObjectTypeCode::SimpleGroup => {
                        let zone = self
                            .zone_manager
                            .resolve_zone(&object_id, req.object_raw)
                            .await?;
                        zone
                    }

                    ObjectTypeCode::Custom => {
                        // 从object_id无法判断是不是zone类型，这里强制当作zone_id来查询一次
                        let zone_id = object_id.clone().try_into().map_err(|e| {
                            let msg = format!(
                                "unknown custom target_id type! target={}, {}",
                                object_id, e
                            );
                            error!("{}", msg);
                            BuckyError::new(BuckyErrorCode::UnSupport, msg)
                        })?;
                        if let Some(zone) = self.zone_manager.query(&zone_id) {
                            zone
                        } else {
                            let msg = format!("zone_id not found or invalid: {}", zone_id);
                            error!("{}", msg);

                            return Err(BuckyError::new(BuckyErrorCode::NotFound, msg));
                        }
                    }

                    _ => {
                        // 其余类型暂不支持
                        let msg = format!(
                            "search zone for object type not support: type={:?}, obj={}",
                            obj_type, object_id
                        );
                        error!("{}", msg);

                        return Err(BuckyError::new(BuckyErrorCode::NotFound, msg));
                    }
                };

                UtilGetZoneInputResponse {
                    zone_id: zone.zone_id(),
                    device_id: zone.ood().to_owned(),
                    zone,
                }
            }
            None => {
                // 没有指定target，那么目标是当前zone和当前zone的ood device
                let info = self.zone_manager.get_current_info().await?;

                UtilGetZoneInputResponse {
                    zone: self.zone_manager.get_current_zone().await.unwrap(),
                    zone_id: info.zone_id.clone(),
                    device_id: info.zone_device_ood_id.clone(),
                }
            }
        };

        Ok(resp)
    }

    pub async fn resolve_ood(
        &self,
        req: UtilResolveOODInputRequest,
    ) -> BuckyResult<UtilResolveOODInputResponse> {
        self.ood_resolver
            .resolve_ood(&req.object_id, req.owner_id)
            .await
            .map(|list| UtilResolveOODInputResponse { device_list: list })
    }

    pub async fn get_ood_status(
        &self,
        req: UtilGetOODStatusInputRequest,
    ) -> BuckyResult<UtilGetOODStatusInputResponse> {
        let sync_client = self.sync_client.get();
        if sync_client.is_none() {
            let msg =
                format!("sync client is not support on ood or not enabled for current device!");
            error!("{}", msg);
            return Err(BuckyError::new(BuckyErrorCode::NotSupport, msg));
        }

        let sync_client = sync_client.unwrap();

        let flush_ping = (req.common.flags | CYFS_ROUTER_REQUEST_FLAG_FLUSH) != 0;
        let status = sync_client.get_ood_status(flush_ping).await?;

        Ok(UtilGetOODStatusInputResponse { status })
    }

    pub async fn get_device_static_info(
        &self,
        _req: UtilGetDeviceStaticInfoInputRequest,
    ) -> BuckyResult<UtilGetDeviceStaticInfoInputResponse> {
        let info = self.zone_manager.get_current_info().await?;

        let device_id = info.device_id.clone();
        let device = self.bdt_stack.sn_client().ping().default_local();
        let owner_id = device.desc().owner().clone();

        let cyfs_root = cyfs_util::get_cyfs_root_path();
        let cyfs_root = cyfs_root
            .to_str()
            .unwrap_or_else(|| {
                error!(
                    "invalid cyfs root path string! root={}",
                    cyfs_root.display()
                );
                ""
            })
            .to_owned();

        let info = DeviceStaticInfo {
            device_id,
            device,

            zone_role: info.zone_role.clone(),
            ood_work_mode: info.ood_work_mode.clone(),
            is_ood_device: info.zone_role.is_ood_device(),
            ood_device_id: info.zone_device_ood_id.clone(),
            zone_id: info.zone_id.clone(),

            root_state_access_mode: self.config.get_access_mode(GlobalStateCategory::RootState),
            local_cache_access_mode: self.config.get_access_mode(GlobalStateCategory::LocalCache),

            owner_id,
            cyfs_root,

            sn_list: self.bdt_stack.sn_client().ping().sn_list().iter().map(|sn| sn.desc().device_id()).collect(),
            known_sn_list: self.bdt_stack.sn_client().cache().known_list(),
        };

        Ok(UtilGetDeviceStaticInfoInputResponse { info })
    }

    pub async fn get_noc_info(
        &self,
        _req: UtilGetNOCInfoInputRequest,
    ) -> BuckyResult<UtilGetNOCInfoInputResponse> {
        let stat = self.noc.stat().await?;

        Ok(UtilGetNOCInfoInputResponse { stat })
    }

    pub async fn get_network_access_info(
        &self,
        _req: UtilGetNetworkAccessInfoInputRequest,
    ) -> BuckyResult<UtilGetNetworkAccessInfoInputResponse> {
        let info = self.access_info_manager.update_access_info()?;

        Ok(UtilGetNetworkAccessInfoInputResponse { info })
    }

    pub async fn get_system_info(
        &self,
        _req: UtilGetSystemInfoInputRequest,
    ) -> BuckyResult<UtilGetSystemInfoInputResponse> {
        let info = SYSTEM_INFO_MANAGER.get_system_info().await;

        Ok(UtilGetSystemInfoInputResponse { info })
    }

    pub async fn get_version_info(
        &self,
        _req: UtilGetVersionInfoInputRequest,
    ) -> BuckyResult<UtilGetVersionInfoInputResponse> {
        let info = VersionInfo {
            version: cyfs_base::get_version().to_owned(),
            channel: cyfs_base::get_channel().to_owned(),
            target: cyfs_base::get_target().to_owned(),
        };

        Ok(UtilGetVersionInfoInputResponse { info })
    }

    pub async fn build_file_object(
        &self,
        req: UtilBuildFileInputRequest,
    ) -> BuckyResult<UtilBuildFileInputResponse> {
        if req.local_path.is_file() {
            let params = BuildFileParams {
                local_path: req.local_path.to_string_lossy().to_string(),
                owner: req.owner,
                dec_id: req.common.source.dec.clone(),
                chunk_size: req.chunk_size, 
                chunk_method: req.chunk_method, 
                access: req.access.map(|v| v.value()),
            };
            let task_id = self
                .task_manager
                .create_task(
                    req.common.source.dec,
                    req.common.source.zone.device.unwrap(),
                    BUILD_FILE_TASK,
                    params,
                )
                .await?;
            self.task_manager.start_task(&task_id).await?;
            self.task_manager.check_and_waiting_stop(&task_id).await;
            let status = BuildFileTaskStatus::clone_from_slice(
                self.task_manager
                    .get_task_detail_status(&task_id)
                    .await?
                    .as_slice(),
            )?;
            if let BuildFileTaskStatus::Finished(file) = status {
                Ok(UtilBuildFileInputResponse {
                    object_id: file.desc().calculate_id(),
                    object_raw: file.to_vec()?,
                })
            } else {
                let msg = format!("build_file_object unexpect status");
                log::error!("{}", msg.as_str());
                Err(BuckyError::new(BuckyErrorCode::Failed, msg))
            }
        } else {
            let params = BuildDirParams {
                local_path: req.local_path.to_string_lossy().to_string(),
                owner: req.owner,
                dec_id: req.common.source.dec.clone(),
                chunk_size: req.chunk_size, 
                chunk_method: req.chunk_method, 
                access: req.access.map(|v|v.value()),
                device_id: self.bdt_stack.local_device_id().object_id().clone(),
            };
            let task_id = self
                .task_manager
                .create_task(
                    req.common.source.dec,
                    req.common.source.zone.device.unwrap(),
                    BUILD_DIR_TASK,
                    params,
                )
                .await?;
            self.task_manager.start_task(&task_id).await?;
            self.task_manager.check_and_waiting_stop(&task_id).await;
            let status = BuildDirTaskStatus::clone_from_slice(
                self.task_manager
                    .get_task_detail_status(&task_id)
                    .await?
                    .as_slice(),
            )?;
            if let BuildDirTaskStatus::Finished(object_id) = status {
                Ok(UtilBuildFileInputResponse {
                    object_id,
                    object_raw: vec![],
                })
            } else {
                let msg = format!("build_file_object unexpect status");
                log::error!("{}", msg.as_str());
                Err(BuckyError::new(BuckyErrorCode::Failed, msg))
            }
        }
    }

    pub async fn build_dir_from_object_map(
        &self,
        req: UtilBuildDirFromObjectMapInputRequest,
    ) -> BuckyResult<UtilBuildDirFromObjectMapInputResponse> {
        let dec_id = req.common.source.dec.clone();

        let noc = ObjectMapNOCCacheAdapter::new_noc_cache(self.noc.clone());
        let root_cache = ObjectMapRootMemoryCache::new_default_ref(Some(dec_id), noc);
        let cache = ObjectMapOpEnvMemoryCache::new_ref(root_cache.clone());

        let dir_id = DirHelper::build_zip_dir_from_object_map(
            &req.common.source,
            &self.noc,
            cache,
            &req.object_map_id,
        )
        .await?;
        Ok(UtilBuildDirFromObjectMapInputResponse { object_id: dir_id })
    }
}

#[async_trait::async_trait]
impl UtilInputProcessor for UtilLocalService {
    async fn get_device(
        &self,
        req: UtilGetDeviceInputRequest,
    ) -> BuckyResult<UtilGetDeviceInputResponse> {
        UtilLocalService::get_device(&self, req).await
    }

    async fn get_zone(
        &self,
        req: UtilGetZoneInputRequest,
    ) -> BuckyResult<UtilGetZoneInputResponse> {
        UtilLocalService::get_zone(&self, req).await
    }

    async fn resolve_ood(
        &self,
        req: UtilResolveOODInputRequest,
    ) -> BuckyResult<UtilResolveOODInputResponse> {
        UtilLocalService::resolve_ood(&self, req).await
    }

    async fn get_ood_status(
        &self,
        req: UtilGetOODStatusInputRequest,
    ) -> BuckyResult<UtilGetOODStatusInputResponse> {
        UtilLocalService::get_ood_status(&self, req).await
    }

    async fn get_noc_info(
        &self,
        req: UtilGetNOCInfoInputRequest,
    ) -> BuckyResult<UtilGetNOCInfoInputResponse> {
        UtilLocalService::get_noc_info(&self, req).await
    }

    async fn get_network_access_info(
        &self,
        req: UtilGetNetworkAccessInfoInputRequest,
    ) -> BuckyResult<UtilGetNetworkAccessInfoInputResponse> {
        UtilLocalService::get_network_access_info(&self, req).await
    }

    async fn get_device_static_info(
        &self,
        req: UtilGetDeviceStaticInfoInputRequest,
    ) -> BuckyResult<UtilGetDeviceStaticInfoInputResponse> {
        UtilLocalService::get_device_static_info(&self, req).await
    }

    async fn get_system_info(
        &self,
        req: UtilGetSystemInfoInputRequest,
    ) -> BuckyResult<UtilGetSystemInfoInputResponse> {
        UtilLocalService::get_system_info(&self, req).await
    }

    async fn get_version_info(
        &self,
        req: UtilGetVersionInfoInputRequest,
    ) -> BuckyResult<UtilGetVersionInfoInputResponse> {
        UtilLocalService::get_version_info(&self, req).await
    }

    async fn build_file_object(
        &self,
        req: UtilBuildFileInputRequest,
    ) -> BuckyResult<UtilBuildFileInputResponse> {
        UtilLocalService::build_file_object(self, req).await
    }

    async fn build_dir_from_object_map(
        &self,
        req: UtilBuildDirFromObjectMapInputRequest,
    ) -> BuckyResult<UtilBuildDirFromObjectMapInputResponse> {
        UtilLocalService::build_dir_from_object_map(self, req).await
    }
}
