use std;
use std::sync::{Arc, Mutex};

use failure::Error;
use futures::Future;
use grpcio::{self, RpcStatus, RpcStatusCode};
use std::fs;
use std::path::Path;
use trow_protobuf;
use trow_protobuf::server::*;
use uuid::Uuid;

/*
 * TODO: figure out what needs to be stored in the backend
 * and what it's keyed on
 * probably need a path
 *
 * remember will probably want to split out metadata for search
 *
 * Accepted Upload is borked atm
 */
/// Struct implementing callbacks for the Frontend
///
/// _uploads_: a HashSet of all uuids that are currently being tracked
#[derive(Clone)]
pub struct BackendService {
    uploads: Arc<Mutex<std::collections::HashSet<Layer>>>,
}

impl BackendService {
    pub fn new() -> Self {
        BackendService {
            uploads: Arc::new(Mutex::new(std::collections::HashSet::new())),
        }
    }
}

#[derive(Eq, PartialEq, Hash, Debug, Clone)]
struct Layer {
    repo_name: String,
    digest: String,
}

//TODO: fix
fn get_path_for_uuid(uuid: &str) -> String {
    format!("data/scratch/{}", uuid)
}

fn save_layer(repo_name: &str, user_digest: &str, uuid: &str) -> Result<(), Error> {
    debug!("Saving layer {}", user_digest);

    //TODO: This is wrong; user digest needs to be verified and potentially changed to our own digest
    //if we want to use consistent compression alg
    let digest_path = format!("data/layers/{}/{}", repo_name, user_digest);
    let path = format!("data/layers/{}", repo_name);
    let scratch_path = format!("data/scratch/{}", uuid);

    if !Path::new(&path).exists() {
        fs::create_dir_all(path)?;
    }

    fs::copy(&scratch_path, digest_path)?;

    //Not an error, even if it's not great
    fs::remove_file(&scratch_path)
        .unwrap_or_else(|e| warn!("Error deleting file {} {:?}", &scratch_path, e));

    Ok(())
}

impl trow_protobuf::server_grpc::Backend for BackendService {
    fn get_write_location_for_blob(
        &self,
        ctx: grpcio::RpcContext,
        blob_ref: BlobRef,
        resp: grpcio::UnarySink<WriteLocation>,
    ) {
        let set = self.uploads.lock().unwrap();
        //LAYER MUST DIE!
        let layer = Layer {
            repo_name: blob_ref.get_repo_name().to_owned(),
            digest: blob_ref.get_uuid().to_owned(),
        };

        if set.contains(&layer) {
            let path = get_path_for_uuid(blob_ref.get_uuid());
            let mut w = WriteLocation::new();
            w.set_path(path);
            let f = resp
                .success(w)
                .map_err(move |e| warn!("Failed sending to client {:?}", e));
            ctx.spawn(f);
        } else {
            let f = resp
                .fail(RpcStatus::new(
                    RpcStatusCode::Unknown,
                    Some("UUID Not Known".to_string()),
                )).map_err(|e| warn!("Received request for unknown UUID {:?}", e));
            ctx.spawn(f);
        }
    }

    fn request_upload(
        &self,
        ctx: grpcio::RpcContext,
        req: UploadRequest,
        sink: grpcio::UnarySink<UploadDetails>,
    ) {
        let mut resp = UploadDetails::new();
        let layer = Layer {
            repo_name: req.get_repo_name().to_owned(),
            //WTF?!
            digest: Uuid::new_v4().to_string(),
        };
        {
            self.uploads.lock().unwrap().insert(layer.clone());
            debug!("Hash Table: {:?}", self.uploads);
        }
        resp.set_uuid(layer.digest.to_owned());
        let f = sink
            .success(resp)
            .map_err(|e| warn!("failed to reply! {:?}", e));
        ctx.spawn(f);
    }

    fn complete_upload(
        &self,
        ctx: grpcio::RpcContext,
        cr: CompleteRequest,
        sink: grpcio::UnarySink<CompletedUpload>,
    ) {

        
        match save_layer(cr.get_repo_name(), cr.get_user_digest(), cr.get_uuid()) {
            Ok(_) => {
                let mut cu = CompletedUpload::new();
                cu.set_digest(cr.get_user_digest().to_string());
                let f = sink
                    .success(cu)
                    .map_err(move |e| warn!("failed to reply! {:?}", e));
                ctx.spawn(f);
            }
            Err(_) => {
                let f = sink
                    .fail(RpcStatus::new(
                        RpcStatusCode::Internal,
                        Some("Internal error saving file".to_string()),
                    )).map_err(|e| warn!("Internal error saving file {:?}", e));
                ctx.spawn(f);
            }
        }

        //delete uuid from uploads tracking
        let layer = Layer {
            repo_name: cr.get_repo_name().to_string(),
            digest: cr.get_user_digest().to_string(),
        };

        let mut set = self.uploads.lock().unwrap();
        set.remove(&layer);
            
    }
}