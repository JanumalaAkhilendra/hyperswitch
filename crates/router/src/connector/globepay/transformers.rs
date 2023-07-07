use error_stack::ResultExt;
use masking::Secret;
use serde::{Deserialize, Serialize};

use crate::{
    core::errors,
    types::{self, api, storage::enums},
};

#[derive(Debug, Serialize)]
pub struct GlobepayPaymentsRequest {
    price: i64,
    description: String,
    currency: enums::Currency,
    channel: GlobepayChannel,
}

#[derive(Debug, Serialize)]
pub enum GlobepayChannel {
    Alipay,
    Wechat,
}

impl TryFrom<&types::PaymentsAuthorizeRouterData> for GlobepayPaymentsRequest {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(item: &types::PaymentsAuthorizeRouterData) -> Result<Self, Self::Error> {
        let channel: GlobepayChannel = match &item.request.payment_method_data {
            api::PaymentMethodData::Wallet(ref wallet_data) => match wallet_data {
                api::WalletData::AliPay(_) => GlobepayChannel::Alipay,
                api::WalletData::WeChatPay(_) => GlobepayChannel::Wechat,
                _ => Err(errors::ConnectorError::NotImplemented(
                    "Payment method".to_string(),
                ))?,
            },
            _ => Err(errors::ConnectorError::NotImplemented(
                "Payment method".to_string(),
            ))?,
        };
        let description =
            item.description
                .clone()
                .ok_or(errors::ConnectorError::MissingRequiredField {
                    field_name: "description",
                })?;
        Ok(Self {
            price: item.request.amount,
            description,
            currency: item.request.currency,
            channel,
        })
    }
}

pub struct GlobepayAuthType {
    pub(super) partner_code: Secret<String>,
    pub(super) credential_code: Secret<String>,
}

impl TryFrom<&types::ConnectorAuthType> for GlobepayAuthType {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(auth_type: &types::ConnectorAuthType) -> Result<Self, Self::Error> {
        match auth_type {
            types::ConnectorAuthType::BodyKey { api_key, key1 } => Ok(Self {
                partner_code: Secret::new(api_key.to_owned()),
                credential_code: Secret::new(key1.to_owned()),
            }),
            _ => Err(errors::ConnectorError::FailedToObtainAuthType.into()),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GlobepayPaymentStatus {
    Success,
    Exists,
}

impl From<GlobepayPaymentStatus> for enums::AttemptStatus {
    fn from(item: GlobepayPaymentStatus) -> Self {
        match item {
            GlobepayPaymentStatus::Success => Self::AuthenticationPending, // this connector only have redirection flows so "Success" is mapped to authenticatoin pending ,ref = "https://pay.globepay.co/docs/en/#api-QRCode-NewQRCode"
            GlobepayPaymentStatus::Exists => Self::Failure,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GlobepayConnectorMetadata {
    image_data_url: url::Url,
}

#[derive(Debug, Deserialize)]
pub struct GlobepayPaymentsResponse {
    result_code: Option<GlobepayPaymentStatus>,
    order_id: Option<String>,
    qrcode_img: Option<url::Url>,
    return_code: GlobepayReturnCode, //Execution result
    return_msg: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, strum::Display)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GlobepayReturnCode {
    Success,
    OrderNotExist,
    OrderMismatch,
    Systemerror,
    InvalidShortId,
    SignTimeout,
    InvalidSign,
    ParamInvalid,
    NotPermitted,
    InvalidChannel,
    DuplicateOrderId,
}

impl<F, T>
    TryFrom<types::ResponseRouterData<F, GlobepayPaymentsResponse, T, types::PaymentsResponseData>>
    for types::RouterData<F, T, types::PaymentsResponseData>
{
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: types::ResponseRouterData<
            F,
            GlobepayPaymentsResponse,
            T,
            types::PaymentsResponseData,
        >,
    ) -> Result<Self, Self::Error> {
        if item.response.return_code == GlobepayReturnCode::Success {
            let globepay_metadata = GlobepayConnectorMetadata {
                image_data_url: item
                    .response
                    .qrcode_img
                    .ok_or(errors::ConnectorError::ResponseHandlingFailed)?,
            };
            let connector_metadata = Some(common_utils::ext_traits::Encode::<
                GlobepayConnectorMetadata,
            >::encode_to_value(&globepay_metadata))
            .transpose()
            .change_context(errors::ConnectorError::ResponseHandlingFailed)?;
            let globepay_status = item
                .response
                .result_code
                .ok_or(errors::ConnectorError::ResponseHandlingFailed)?;

            Ok(Self {
                status: enums::AttemptStatus::from(globepay_status),
                response: Ok(types::PaymentsResponseData::TransactionResponse {
                    resource_id: types::ResponseId::ConnectorTransactionId(
                        item.response
                            .order_id
                            .ok_or(errors::ConnectorError::ResponseHandlingFailed)?,
                    ),
                    redirection_data: None,
                    mandate_reference: None,
                    connector_metadata,
                    network_txn_id: None,
                }),
                ..item.data
            })
        } else {
            Ok(Self {
                status: enums::AttemptStatus::Failure, //As this connector gives 200 in failed scenarios . if return_code is not success status is mapped to failure. ref = "https://pay.globepay.co/docs/en/#api-QRCode-NewQRCode"
                response: Err(types::ErrorResponse {
                    code: item.response.return_code.to_string(),
                    message: item.response.return_code.to_string(),
                    reason: item.response.return_msg,
                    status_code: item.http_code,
                }),
                ..item.data
            })
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct GlobepaySyncResponse {
    pub result_code: Option<GlobepayPaymentPsyncStatus>,
    pub order_id: Option<String>,
    pub return_code: GlobepayReturnCode,
    pub return_msg: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GlobepayPaymentPsyncStatus {
    Paying,
    CreateFail,
    Closed,
    PayFail,
    PaySuccess,
}

impl From<GlobepayPaymentPsyncStatus> for enums::AttemptStatus {
    fn from(item: GlobepayPaymentPsyncStatus) -> Self {
        match item {
            GlobepayPaymentPsyncStatus::PaySuccess => Self::Charged,
            GlobepayPaymentPsyncStatus::PayFail
            | GlobepayPaymentPsyncStatus::CreateFail
            | GlobepayPaymentPsyncStatus::Closed => Self::Failure,
            GlobepayPaymentPsyncStatus::Paying => Self::AuthenticationPending,
        }
    }
}

impl<F, T>
    TryFrom<types::ResponseRouterData<F, GlobepaySyncResponse, T, types::PaymentsResponseData>>
    for types::RouterData<F, T, types::PaymentsResponseData>
{
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: types::ResponseRouterData<F, GlobepaySyncResponse, T, types::PaymentsResponseData>,
    ) -> Result<Self, Self::Error> {
        if item.response.return_code == GlobepayReturnCode::Success {
            let globepay_status = item
                .response
                .result_code
                .ok_or(errors::ConnectorError::ResponseHandlingFailed)?;
            let globepay_id = item
                .response
                .order_id
                .ok_or(errors::ConnectorError::ResponseHandlingFailed)?;
            Ok(Self {
                status: enums::AttemptStatus::from(globepay_status),
                response: Ok(types::PaymentsResponseData::TransactionResponse {
                    resource_id: types::ResponseId::ConnectorTransactionId(globepay_id),
                    redirection_data: None,
                    mandate_reference: None,
                    connector_metadata: None,
                    network_txn_id: None,
                }),
                ..item.data
            })
        } else {
            Ok(Self {
                status: enums::AttemptStatus::Failure,
                response: Err(types::ErrorResponse {
                    code: item.response.return_code.to_string(),
                    message: item.response.return_code.to_string(),
                    reason: item.response.return_msg,
                    status_code: item.http_code,
                }),
                ..item.data
            })
        }
    }
}

#[derive(Debug, Serialize)]
pub struct GlobepayRefundRequest {
    pub amount: i64,
}

impl<F> TryFrom<&types::RefundsRouterData<F>> for GlobepayRefundRequest {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(item: &types::RefundsRouterData<F>) -> Result<Self, Self::Error> {
        Ok(Self {
            amount: item.request.refund_amount,
        })
    }
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Default, Deserialize, Clone)]
pub enum RefundStatus {
    Succeeded,
    Failed,
    #[default]
    Processing,
}

impl From<RefundStatus> for enums::RefundStatus {
    fn from(item: RefundStatus) -> Self {
        match item {
            RefundStatus::Succeeded => Self::Success,
            RefundStatus::Failed => Self::Failure,
            RefundStatus::Processing => Self::Pending,
        }
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct RefundResponse {
    id: String,
    status: RefundStatus,
}

impl TryFrom<types::RefundsResponseRouterData<api::Execute, RefundResponse>>
    for types::RefundsRouterData<api::Execute>
{
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: types::RefundsResponseRouterData<api::Execute, RefundResponse>,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            response: Ok(types::RefundsResponseData {
                connector_refund_id: item.response.id.to_string(),
                refund_status: enums::RefundStatus::from(item.response.status),
            }),
            ..item.data
        })
    }
}

impl TryFrom<types::RefundsResponseRouterData<api::RSync, RefundResponse>>
    for types::RefundsRouterData<api::RSync>
{
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: types::RefundsResponseRouterData<api::RSync, RefundResponse>,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            response: Ok(types::RefundsResponseData {
                connector_refund_id: item.response.id.to_string(),
                refund_status: enums::RefundStatus::from(item.response.status),
            }),
            ..item.data
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct GlobepayErrorResponse {
    pub return_msg: String,
    pub return_code: GlobepayReturnCode,
    pub message: String,
}